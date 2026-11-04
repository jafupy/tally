use std::ops::AddAssign;

#[derive(Default, Clone, Copy, serde::Serialize)]
pub struct Stats {
    pub files: u64,
    pub lines: u64,
    pub comments: u64,
    pub blanks: u64,
    pub code: u64,
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
