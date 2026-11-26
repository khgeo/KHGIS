//! Writes a new SQLite database file in pure Rust, so the browser can make a GeoPackage too (the
//! desktop's GeoPackage writer uses the SQLite library, which the browser build doesn't link).
//!
//! It writes what a complete, fresh database holds and nothing more: tables with their rows,
//! each table's B-tree built bottom-up from rows sorted by rowid, payloads that don't fit a page
//! continued on overflow pages, and the automatic indexes SQLite keeps for `UNIQUE` and
//! non-integer `PRIMARY KEY` constraints, so the constraints hold when another program (QGIS,
//! GDAL) adds rows later. There are no free pages, triggers or user indexes. The format is the
//! one documented at <https://www.sqlite.org/fileformat2.html>.

use std::cmp::Ordering;

/// Page size: 4096, SQLite's default.
const PAGE: usize = 4096;
/// The usable size of a page (no reserved bytes).
const U: usize = PAGE;
/// The most pages a database is written with (16 GiB): more is refused rather than attempted.
const MAX_PAGES: usize = 4_000_000;

/// A value in a row.
#[derive(Clone, Debug, PartialEq)]
pub enum Sql {
    Null,
    Int(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

/// A table to write.
#[derive(Clone, Debug, Default)]
pub struct Table {
    pub name: String,
    /// Its `CREATE TABLE` statement, as `sqlite_master` keeps it.
    pub sql: String,
    /// Rows by rowid; a column that is the table's `INTEGER PRIMARY KEY` holds `Null` here.
    pub rows: Vec<(i64, Vec<Sql>)>,
    /// The column sets SQLite indexes automatically (`UNIQUE`, a non-integer `PRIMARY KEY`), in
    /// the order the constraints appear in `sql`.
    pub unique: Vec<Vec<usize>>,
}

/// Database-wide settings kept in the file header.
#[derive(Clone, Copy, Debug, Default)]
pub struct Header {
    /// `PRAGMA application_id` (a GeoPackage's is `0x47504B47`, "GPKG").
    pub application_id: u32,
    /// `PRAGMA user_version` (a GeoPackage 1.3's is 10300).
    pub user_version: u32,
}

fn varint(mut v: u64, out: &mut Vec<u8>) {
    if v > 0x00ff_ffff_ffff_ffff {
        let mut buf = [0u8; 9];
        buf[8] = v as u8;
        v >>= 8;
        for i in (0..8).rev() {
            buf[i] = ((v & 0x7f) as u8) | 0x80;
            v >>= 7;
        }
        out.extend_from_slice(&buf);
        return;
    }
    let mut tmp = [0u8; 9];
    let mut n = 0;
    loop {
        tmp[n] = ((v & 0x7f) as u8) | 0x80;
        n += 1;
        v >>= 7;
        if v == 0 {
            break;
        }
    }
    tmp[0] &= 0x7f;
    for i in (0..n).rev() {
        out.push(tmp[i]);
    }
}

/// A row as a record: its header (serial types) then the values.
fn record(values: &[Sql]) -> Vec<u8> {
    let mut types = Vec::new();
    let mut body = Vec::new();
    for v in values {
        let t: u64 = match v {
            Sql::Null => 0,
            Sql::Int(0) => 8,
            Sql::Int(1) => 9,
            Sql::Int(i) => {
                let i = *i;
                let (t, n) = if (-128..128).contains(&i) {
                    (1, 1)
                } else if (-32768..32768).contains(&i) {
                    (2, 2)
                } else if (-8_388_608..8_388_608).contains(&i) {
                    (3, 3)
                } else if (-2_147_483_648..2_147_483_648).contains(&i) {
                    (4, 4)
                } else if (-140_737_488_355_328..140_737_488_355_328).contains(&i) {
                    (5, 6)
                } else {
                    (6, 8)
                };
                body.extend_from_slice(&i.to_be_bytes()[8 - n..]);
                t
            }
            Sql::Real(f) => {
                body.extend_from_slice(&f.to_be_bytes());
                7
            }
            Sql::Text(s) => {
                body.extend_from_slice(s.as_bytes());
                (s.len() as u64) * 2 + 13
            }
            Sql::Blob(b) => {
                body.extend_from_slice(b);
                (b.len() as u64) * 2 + 12
            }
        };
        varint(t, &mut types);
    }
    // The header's size counts itself: one byte more when that crosses a varint boundary.
    let mut size = types.len() + 1;
    let mut head = Vec::new();
    loop {
        head.clear();
        varint(size as u64, &mut head);
        if head.len() + types.len() == size {
            break;
        }
        size = head.len() + types.len();
    }
    head.extend_from_slice(&types);
    head.extend_from_slice(&body);
    head
}

/// The pages being written, numbered from 1.
struct Pages {
    pages: Vec<Vec<u8>>,
}

impl Pages {
    fn alloc(&mut self) -> Result<u32, String> {
        if self.pages.len() >= MAX_PAGES {
            return Err("the database would be too large".into());
        }
        self.pages.push(vec![0; PAGE]);
        Ok(self.pages.len() as u32)
    }

    fn set(&mut self, n: u32, page: Vec<u8>) {
        if let Some(p) = self.pages.get_mut(n as usize - 1) {
            *p = page;
        }
    }

    /// Splits a cell's payload: the part kept on the page, then overflow pages for the rest.
    /// `max_local` is the most a cell keeps on its page before overflowing (`X` in the format).
    fn spill(&mut self, payload: &[u8], max_local: usize) -> Result<(Vec<u8>, Option<u32>), String> {
        if payload.len() <= max_local {
            return Ok((payload.to_vec(), None));
        }
        let m = ((U - 12) * 32 / 255) - 23;
        let k = m + ((payload.len() - m) % (U - 4));
        let local = if k <= max_local { k } else { m };
        let (here, rest) = payload.split_at(local);
        // The overflow chain, written last page first so each knows the next.
        let chunks: Vec<&[u8]> = rest.chunks(U - 4).collect();
        let mut next = 0u32;
        let mut numbers = Vec::with_capacity(chunks.len());
        for _ in &chunks {
            numbers.push(self.alloc()?);
        }
        for (i, chunk) in chunks.iter().enumerate().rev() {
            let mut page = vec![0u8; PAGE];
            page[..4].copy_from_slice(&next.to_be_bytes());
            page[4..4 + chunk.len()].copy_from_slice(chunk);
            self.set(numbers[i], page);
            next = numbers[i];
        }
        Ok((here.to_vec(), numbers.first().copied()))
    }
}

/// A B-tree page being filled: its cells, and where its header starts (100 on page 1).
struct Node {
    kind: u8,
    cells: Vec<Vec<u8>>,
    used: usize,
    offset: usize,
}

impl Node {
    fn new(kind: u8, offset: usize) -> Node {
        Node { kind, cells: Vec::new(), used: 0, offset }
    }
    fn header_len(&self) -> usize {
        if self.kind == 0x05 || self.kind == 0x02 { 12 } else { 8 }
    }
    fn fits(&self, cell: usize) -> bool {
        self.offset + self.header_len() + 2 * (self.cells.len() + 1) + self.used + cell <= PAGE
    }
    fn push(&mut self, cell: Vec<u8>) {
        self.used += cell.len();
        self.cells.push(cell);
    }
    /// The page's bytes (an interior page's right-most child given).
    fn bytes(&self, right: Option<u32>, mut base: Vec<u8>) -> Vec<u8> {
        base.resize(PAGE, 0);
        let h = self.offset;
        base[h] = self.kind;
        base[h + 1..h + 3].copy_from_slice(&0u16.to_be_bytes());
        base[h + 3..h + 5].copy_from_slice(&(self.cells.len() as u16).to_be_bytes());
        let mut end = PAGE;
        let mut ptr = h + self.header_len();
        for c in &self.cells {
            end -= c.len();
            base[end..end + c.len()].copy_from_slice(c);
            base[ptr..ptr + 2].copy_from_slice(&(end as u16).to_be_bytes());
            ptr += 2;
        }
        // A content area starting at 65536 is written as 0.
        let start = if end == 65536 { 0 } else { end as u16 };
        base[h + 5..h + 7].copy_from_slice(&start.to_be_bytes());
        base[h + 7] = 0;
        if let Some(r) = right {
            base[h + 8..h + 12].copy_from_slice(&r.to_be_bytes());
        }
        base
    }
}

/// Writes a table B-tree of `rows` (sorted by rowid); its root goes on page `root` (allocated
/// by the caller, so `sqlite_master` can be rooted on page 1).
fn table_tree(pages: &mut Pages, root: u32, rows: &[(i64, Vec<u8>)]) -> Result<(), String> {
    let root_offset = if root == 1 { 100 } else { 0 };
    let max_local = U - 35;
    // The leaves: (page number, largest rowid on it).
    let mut level: Vec<(u32, i64)> = Vec::new();
    let mut node = Node::new(0x0D, 0);
    let mut last = i64::MIN;
    let mut built: Vec<(Node, i64)> = Vec::new();
    for (rowid, payload) in rows {
        let (local, overflow) = pages.spill(payload, max_local)?;
        let mut cell = Vec::with_capacity(local.len() + 16);
        varint(payload.len() as u64, &mut cell);
        varint(*rowid as u64, &mut cell);
        cell.extend_from_slice(&local);
        if let Some(o) = overflow {
            cell.extend_from_slice(&o.to_be_bytes());
        }
        if !node.fits(cell.len()) {
            built.push((std::mem::replace(&mut node, Node::new(0x0D, 0)), last));
        }
        node.push(cell);
        last = *rowid;
    }
    built.push((node, last));
    // One leaf on page 1 must also leave room for the file header: split it in two if not.
    if built.len() == 1 && root_offset > 0 {
        let need = built.first().map_or(0, |(leaf, _)| root_offset + 8 + 2 * leaf.cells.len() + leaf.used);
        if need > PAGE && built.first().is_some_and(|(leaf, _)| leaf.cells.len() > 1) {
            let (leaf, key) = built.remove(0);
            let half = leaf.cells.len() / 2;
            let mut a = Node::new(0x0D, 0);
            let mut b = Node::new(0x0D, 0);
            // The rowid of the last cell of the first half: read back from the cell.
            let split_key = rows.get(half - 1).map(|r| r.0).unwrap_or(key);
            for (i, c) in leaf.cells.into_iter().enumerate() {
                if i < half { a.push(c) } else { b.push(c) }
            }
            built.push((a, split_key));
            built.push((b, key));
        }
    }
    // One leaf: it is the root.
    if built.len() == 1 {
        let (mut leaf, _) = built.remove(0);
        leaf.offset = root_offset;
        let page = leaf.bytes(None, take_page(pages, root));
        pages.set(root, page);
        return Ok(());
    }
    for (leaf, key) in built {
        let n = pages.alloc()?;
        pages.set(n, leaf.bytes(None, vec![0; PAGE]));
        level.push((n, key));
    }
    // Interior levels, until one page holds them all. An interior cell is at most 13 bytes and
    // its pointer 2, so a page holds `cap` cells plus its right-most child.
    let cap = |offset: usize| (PAGE - offset - 12) / 15;
    loop {
        if level.len() <= cap(root_offset) + 1 {
            let mut top = Node::new(0x05, root_offset);
            let (right, _) = *level.last().ok_or("an empty B-tree level")?;
            for (child, key) in &level[..level.len() - 1] {
                let mut cell = child.to_be_bytes().to_vec();
                varint(*key as u64, &mut cell);
                top.push(cell);
            }
            let page = top.bytes(Some(right), take_page(pages, root));
            pages.set(root, page);
            return Ok(());
        }
        // Groups of nearly equal size, each a page: its last child is the right-most.
        let per = cap(0) + 1;
        let groups = level.len().div_ceil(per);
        let size = level.len().div_ceil(groups);
        let mut next: Vec<(u32, i64)> = Vec::new();
        for group in level.chunks(size) {
            let mut node = Node::new(0x05, 0);
            let (right, key) = *group.last().ok_or("an empty group")?;
            for (child, k) in &group[..group.len() - 1] {
                let mut cell = child.to_be_bytes().to_vec();
                varint(*k as u64, &mut cell);
                node.push(cell);
            }
            let n = pages.alloc()?;
            pages.set(n, node.bytes(Some(right), vec![0; PAGE]));
            next.push((n, key));
        }
        level = next;
    }
}

/// Page `n`'s current bytes (page 1 keeps its file header).
fn take_page(pages: &Pages, n: u32) -> Vec<u8> {
    pages.pages.get(n as usize - 1).cloned().unwrap_or_else(|| vec![0; PAGE])
}

/// The length of the varint at the start of `b`, into `n`.
fn varint_len(b: &[u8], n: &mut usize) {
    *n = b.iter().take(9).position(|x| x & 0x80 == 0).map_or(9, |i| i + 1);
}

/// SQLite's order of values in an index (BINARY collation): NULL, numbers, text, blobs.
fn compare(a: &Sql, b: &Sql) -> Ordering {
    let rank = |v: &Sql| match v {
        Sql::Null => 0,
        Sql::Int(_) | Sql::Real(_) => 1,
        Sql::Text(_) => 2,
        Sql::Blob(_) => 3,
    };
    match (a, b) {
        (Sql::Int(x), Sql::Int(y)) => x.cmp(y),
        (Sql::Int(x), Sql::Real(y)) => (*x as f64).partial_cmp(y).unwrap_or(Ordering::Equal),
        (Sql::Real(x), Sql::Int(y)) => x.partial_cmp(&(*y as f64)).unwrap_or(Ordering::Equal),
        (Sql::Real(x), Sql::Real(y)) => x.partial_cmp(y).unwrap_or(Ordering::Equal),
        (Sql::Text(x), Sql::Text(y)) => x.as_bytes().cmp(y.as_bytes()),
        (Sql::Blob(x), Sql::Blob(y)) => x.cmp(y),
        _ => rank(a).cmp(&rank(b)),
    }
}

/// Writes an index B-tree of `keys` (sorted); its root on page `root`. Index entries are short
/// here (table and column names): an entry too long for a page is refused.
fn index_tree(pages: &mut Pages, root: u32, keys: &[Vec<u8>]) -> Result<(), String> {
    let max_local = ((U - 12) * 64 / 255) - 23;
    let cell_of = |k: &Vec<u8>| -> Result<Vec<u8>, String> {
        if k.len() > max_local {
            return Err("an index entry too long for this writer".into());
        }
        let mut c = Vec::with_capacity(k.len() + 4);
        varint(k.len() as u64, &mut c);
        c.extend_from_slice(k);
        Ok(c)
    };
    // Leaves, with the entry after each full leaf held back as the separator above it.
    let mut leaves: Vec<Node> = Vec::new();
    let mut separators: Vec<Vec<u8>> = Vec::new();
    let mut node = Node::new(0x0A, 0);
    let mut iter = keys.iter().peekable();
    while let Some(k) = iter.next() {
        let cell = cell_of(k)?;
        if node.fits(cell.len()) {
            node.push(cell);
            continue;
        }
        // Full. With entries after it, this one goes up as the separator and the next start a
        // new leaf; as the last entry, it starts the new leaf and the full leaf's own last
        // entry goes up instead.
        let mut full = std::mem::replace(&mut node, Node::new(0x0A, 0));
        if iter.peek().is_some() {
            separators.push(k.clone());
        } else {
            let last = full.cells.pop().ok_or("an empty index leaf")?;
            full.used -= last.len();
            let mut skip = 0;
            varint_len(&last, &mut skip);
            separators.push(last.get(skip..).unwrap_or(&[]).to_vec());
            node.push(cell);
        }
        leaves.push(full);
    }
    leaves.push(node);
    let root_offset = if root == 1 { 100 } else { 0 };
    if leaves.len() == 1 {
        let mut leaf = leaves.remove(0);
        leaf.offset = root_offset;
        let page = leaf.bytes(None, take_page(pages, root));
        pages.set(root, page);
        return Ok(());
    }
    let mut children = Vec::new();
    for leaf in leaves {
        let n = pages.alloc()?;
        pages.set(n, leaf.bytes(None, vec![0; PAGE]));
        children.push(n);
    }
    let mut top = Node::new(0x02, root_offset);
    for (i, sep) in separators.iter().enumerate() {
        let mut cell = children[i].to_be_bytes().to_vec();
        let c = cell_of(sep)?;
        cell.extend_from_slice(&c);
        if !top.fits(cell.len()) {
            return Err("too many index entries for this writer".into());
        }
        top.push(cell);
    }
    let right = *children.last().ok_or("an empty index")?;
    let page = top.bytes(Some(right), take_page(pages, root));
    pages.set(root, page);
    Ok(())
}

/// The database file holding `tables`.
pub fn database(tables: &[Table], header: Header) -> Result<Vec<u8>, String> {
    let mut pages = Pages { pages: Vec::new() };
    pages.alloc()?; // page 1: the file header and sqlite_master's root
    let mut master: Vec<(i64, Vec<u8>)> = Vec::new();
    for t in tables {
        let root = pages.alloc()?;
        let mut rows: Vec<(i64, Vec<u8>)> = t.rows.iter().map(|(id, vals)| (*id, record(vals))).collect();
        rows.sort_by_key(|r| r.0);
        if rows.windows(2).any(|w| w[0].0 == w[1].0) {
            return Err(format!("{}: two rows have the same rowid", t.name));
        }
        table_tree(&mut pages, root, &rows)?;
        let id = master.len() as i64 + 1;
        master.push((
            id,
            record(&[
                Sql::Text("table".into()),
                Sql::Text(t.name.clone()),
                Sql::Text(t.name.clone()),
                Sql::Int(root as i64),
                Sql::Text(t.sql.clone()),
            ]),
        ));
        for (k, cols) in t.unique.iter().enumerate() {
            let iroot = pages.alloc()?;
            let mut entries: Vec<(Vec<Sql>, i64)> =
                t.rows.iter().map(|(id, vals)| (cols.iter().map(|c| vals.get(*c).cloned().unwrap_or(Sql::Null)).collect(), *id)).collect();
            entries.sort_by(|a, b| {
                a.0.iter()
                    .zip(&b.0)
                    .map(|(x, y)| compare(x, y))
                    .find(|o| *o != Ordering::Equal)
                    .unwrap_or(Ordering::Equal)
                    .then(a.1.cmp(&b.1))
            });
            if entries
                .windows(2)
                .any(|w| w[0].0.iter().zip(&w[1].0).all(|(x, y)| compare(x, y) == Ordering::Equal) && !w[0].0.contains(&Sql::Null))
            {
                return Err(format!("{}: a value meant to be unique repeats", t.name));
            }
            let keys: Vec<Vec<u8>> = entries
                .into_iter()
                .map(|(mut vals, id)| {
                    vals.push(Sql::Int(id));
                    record(&vals)
                })
                .collect();
            index_tree(&mut pages, iroot, &keys)?;
            let name = format!("sqlite_autoindex_{}_{}", t.name, k + 1);
            let id = master.len() as i64 + 1;
            master.push((
                id,
                record(&[Sql::Text("index".into()), Sql::Text(name), Sql::Text(t.name.clone()), Sql::Int(iroot as i64), Sql::Null]),
            ));
        }
    }
    table_tree(&mut pages, 1, &master)?;
    // The file header, on page 1.
    let n = pages.pages.len() as u32;
    let first = pages.pages.first_mut().ok_or("no first page")?;
    first[..16].copy_from_slice(b"SQLite format 3\0");
    first[16..18].copy_from_slice(&(PAGE as u16).to_be_bytes());
    first[18] = 1;
    first[19] = 1;
    first[20] = 0;
    first[21] = 64;
    first[22] = 32;
    first[23] = 32;
    first[24..28].copy_from_slice(&1u32.to_be_bytes()); // change counter
    first[28..32].copy_from_slice(&n.to_be_bytes());
    first[32..40].fill(0); // no free pages
    first[40..44].copy_from_slice(&1u32.to_be_bytes()); // schema cookie
    first[44..48].copy_from_slice(&4u32.to_be_bytes()); // schema format
    first[48..56].fill(0);
    first[56..60].copy_from_slice(&1u32.to_be_bytes()); // UTF-8
    first[60..64].copy_from_slice(&header.user_version.to_be_bytes());
    first[64..68].fill(0);
    first[68..72].copy_from_slice(&header.application_id.to_be_bytes());
    first[72..92].fill(0);
    first[92..96].copy_from_slice(&1u32.to_be_bytes()); // version-valid-for
    first[96..100].copy_from_slice(&3_045_000u32.to_be_bytes());
    Ok(pages.pages.concat())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varints_and_records() {
        let mut v = Vec::new();
        varint(300, &mut v);
        assert_eq!(v, [0x82, 0x2C]);
        let r = record(&[Sql::Null, Sql::Int(1), Sql::Text("ab".into())]);
        assert_eq!(r, [4, 0, 9, 17, b'a', b'b']);
    }

    #[test]
    fn a_large_database_is_written_whole() {
        let rows: Vec<(i64, Vec<Sql>)> =
            (1..=5000).map(|i| (i, vec![Sql::Null, Sql::Text(format!("name {i}")), Sql::Blob(vec![7; (i as usize % 9) * 1500])])).collect();
        let t = Table {
            name: "t".into(),
            sql: "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT UNIQUE, b BLOB)".into(),
            rows,
            unique: vec![vec![1]],
        };
        let bytes = database(&[t], Header::default()).unwrap();
        // KHGIS_SQLITE_OUT=path keeps the file, for checking it with SQLite itself.
        if let Some(out) = std::env::var_os("KHGIS_SQLITE_OUT") {
            std::fs::write(out, &bytes).unwrap();
        }
        assert_eq!(&bytes[..16], b"SQLite format 3\0");
        assert_eq!(bytes.len() % PAGE, 0);
        assert!(
            database(
                &[Table { name: "d".into(), sql: String::new(), rows: vec![(1, vec![]), (1, vec![])], unique: vec![] }],
                Header::default()
            )
            .is_err()
        );
    }
}
