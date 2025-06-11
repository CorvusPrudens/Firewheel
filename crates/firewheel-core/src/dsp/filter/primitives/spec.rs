pub trait FilterSpec {
    const ORDER: usize;
    const NUM_SVF: usize = Self::ORDER / 2;
    const NEEDS_ONE_POLE: bool = Self::ORDER % 2 == 1;
}

pub struct DbOct6;
pub struct DbOct12;
pub struct DbOct18;
pub struct DbOct24;
pub struct DbOct30;
pub struct DbOct36;
pub struct DbOct42;
pub struct DbOct48;
pub struct DbOct54;
pub struct DbOct60;
pub struct DbOct66;
pub struct DbOct72;
pub struct DbOct78;
pub struct DbOct84;
pub struct DbOct90;
pub struct DbOct96;

impl FilterSpec for DbOct6 {
    const ORDER: usize = 1;
}
impl FilterSpec for DbOct12 {
    const ORDER: usize = 2;
}
impl FilterSpec for DbOct18 {
    const ORDER: usize = 3;
}
impl FilterSpec for DbOct24 {
    const ORDER: usize = 4;
}
impl FilterSpec for DbOct30 {
    const ORDER: usize = 5;
}
impl FilterSpec for DbOct36 {
    const ORDER: usize = 6;
}
impl FilterSpec for DbOct42 {
    const ORDER: usize = 7;
}
impl FilterSpec for DbOct48 {
    const ORDER: usize = 8;
}
impl FilterSpec for DbOct54 {
    const ORDER: usize = 9;
}
impl FilterSpec for DbOct60 {
    const ORDER: usize = 10;
}
impl FilterSpec for DbOct66 {
    const ORDER: usize = 11;
}
impl FilterSpec for DbOct72 {
    const ORDER: usize = 12;
}
impl FilterSpec for DbOct78 {
    const ORDER: usize = 13;
}
impl FilterSpec for DbOct84 {
    const ORDER: usize = 14;
}
impl FilterSpec for DbOct90 {
    const ORDER: usize = 15;
}
impl FilterSpec for DbOct96 {
    const ORDER: usize = 16;
}
