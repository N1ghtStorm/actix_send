// state of interval futures and context
pub(crate) const SHUT: usize = 1 << 0;
pub(crate) const SLEEP: usize = 1 << 1;
pub(crate) const ACTIVE: usize = 1 << 2;
