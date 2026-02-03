pub struct VirtualTableModel {
    _private: (),
}

impl VirtualTableModel {
    pub fn new(rows: usize, columns: usize, seed: u64) -> Self {
        let _ = (rows, columns, seed);
        Self { _private: () }
    }
}
