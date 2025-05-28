use super::{
    cascade::{CascadeEven, CascadeOdd},
    filter_trait::Filter,
};

pub trait Steepness: Default {
    const ORDER: usize;
    type ConcreteFilter: Filter;
}

pub trait EvenOrderSteepness {}
pub trait OddOrderSteepness {}

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
    type ConcreteFilter = CascadeOdd<1>;
}
impl Steepness for DbOct18 {
    const ORDER: usize = 2;
    type ConcreteFilter = CascadeOdd<2>;
}
impl OddOrderSteepness for DbOct6 {}
impl OddOrderSteepness for DbOct18 {}

impl Steepness for DbOct12 {
    const ORDER: usize = 2;
    type ConcreteFilter = CascadeEven<2>;
}
impl Steepness for DbOct24 {
    const ORDER: usize = 3;
    type ConcreteFilter = CascadeEven<3>;
}
impl Steepness for DbOct36 {
    const ORDER: usize = 4;
    type ConcreteFilter = CascadeEven<4>;
}
impl Steepness for DbOct48 {
    const ORDER: usize = 5;
    type ConcreteFilter = CascadeEven<5>;
}
impl Steepness for DbOct60 {
    const ORDER: usize = 6;
    type ConcreteFilter = CascadeEven<6>;
}
impl Steepness for DbOct72 {
    const ORDER: usize = 7;
    type ConcreteFilter = CascadeEven<7>;
}
impl Steepness for DbOct84 {
    const ORDER: usize = 8;
    type ConcreteFilter = CascadeEven<8>;
}
impl Steepness for DbOct96 {
    const ORDER: usize = 9;
    type ConcreteFilter = CascadeEven<9>;
}
impl EvenOrderSteepness for DbOct12 {}
impl EvenOrderSteepness for DbOct24 {}
impl EvenOrderSteepness for DbOct36 {}
impl EvenOrderSteepness for DbOct48 {}
impl EvenOrderSteepness for DbOct60 {}
impl EvenOrderSteepness for DbOct72 {}
impl EvenOrderSteepness for DbOct84 {}
impl EvenOrderSteepness for DbOct96 {}
