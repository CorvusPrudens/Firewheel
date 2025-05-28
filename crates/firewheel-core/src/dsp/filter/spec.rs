pub trait Steepness: Default {
    const ORDER: usize;
    const DOUBLE_ORDER: usize;
}

#[derive(Default)]
pub struct DbOct6;
#[derive(Default)]
pub struct DbOct12;
#[derive(Default)]
pub struct DbOct18;
#[derive(Default)]
pub struct DbOct24;
#[derive(Default)]
pub struct DbOct36;
#[derive(Default)]
pub struct DbOct48;
#[derive(Default)]
pub struct DbOct60;
#[derive(Default)]
pub struct DbOct72;
#[derive(Default)]
pub struct DbOct84;
#[derive(Default)]
pub struct DbOct96;

impl Steepness for DbOct6 {
    const ORDER: usize = 1;
    const DOUBLE_ORDER: usize = 2;
}
impl Steepness for DbOct12 {
    const ORDER: usize = 2;
    const DOUBLE_ORDER: usize = 4;
}
impl Steepness for DbOct18 {
    const ORDER: usize = 2;
    const DOUBLE_ORDER: usize = 4;
}
impl Steepness for DbOct24 {
    const ORDER: usize = 3;
    const DOUBLE_ORDER: usize = 6;
}
impl Steepness for DbOct36 {
    const ORDER: usize = 4;
    const DOUBLE_ORDER: usize = 8;
}
impl Steepness for DbOct48 {
    const ORDER: usize = 5;
    const DOUBLE_ORDER: usize = 10;
}
impl Steepness for DbOct60 {
    const ORDER: usize = 6;
    const DOUBLE_ORDER: usize = 12;
}
impl Steepness for DbOct72 {
    const ORDER: usize = 7;
    const DOUBLE_ORDER: usize = 14;
}
impl Steepness for DbOct84 {
    const ORDER: usize = 8;
    const DOUBLE_ORDER: usize = 16;
}
impl Steepness for DbOct96 {
    const ORDER: usize = 9;
    const DOUBLE_ORDER: usize = 18;
}
