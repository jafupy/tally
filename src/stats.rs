use std::ops::AddAssign;

#[derive(Clone, Copy, Default)]
pub struct Stats {
    pub(crate) files: u64,
    pub(crate) lines: u64,
    pub(crate) comments: u64,
    pub(crate) blanks: u64,
    pub(crate) code: u64,
}

impl AddAssign for Stats {
    fn add_assign(&mut self, other: Self) {
        self.files += other.files;
        self.lines += other.lines;
        self.comments += other.comments;
        self.blanks += other.blanks;
        self.code += other.code;
    }
}
