use glam::{Vec2, Vec3};

use super::PatchError;
use crate::event::{NodeEventType, ParamData};

pub trait Update {
    type Update;

    fn update(data: &ParamData, path: &[u32]) -> Result<Self::Update, PatchError>;

    fn update_event(event: &NodeEventType) -> Option<Self::Update> {
        match event {
            NodeEventType::Param { data, path } => Self::update(data, path).ok(),
            _ => None,
        }
    }
}

macro_rules! primitive_update {
    ($ty:ty, $variant:ident) => {
        impl Update for $ty {
            type Update = $ty;

            fn update(data: &ParamData, _: &[u32]) -> Result<Self::Update, PatchError> {
                match data {
                    ParamData::$variant(data) => Ok(*data as $ty),
                    _ => Err(PatchError::InvalidData),
                }
            }
        }
    };
}

primitive_update!(bool, Bool);

primitive_update!(u8, U32);
primitive_update!(u16, U32);
primitive_update!(u32, U32);
primitive_update!(u64, U64);

primitive_update!(i8, I32);
primitive_update!(i16, I32);
primitive_update!(i32, I32);
primitive_update!(i64, U64);

primitive_update!(f32, F32);
primitive_update!(f64, F64);

impl Update for Vec2 {
    type Update = Vec2;

    fn update(data: &ParamData, _: &[u32]) -> Result<Self::Update, PatchError> {
        data.try_into()
    }
}

impl Update for Vec3 {
    type Update = Vec3;

    fn update(data: &ParamData, _: &[u32]) -> Result<Self::Update, PatchError> {
        data.try_into()
    }
}

struct UpdateTest {
    a: f32,
    b: bool,
}

pub enum UpdateTestUpdate {
    A(<f32 as Update>::Update),
    B(<bool as Update>::Update),
}

impl Update for UpdateTest {
    type Update = UpdateTestUpdate;

    fn update(data: &ParamData, path: &[u32]) -> Result<Self::Update, PatchError> {
        match path {
            [0, tail @ ..] => Ok(UpdateTestUpdate::A {
                0: f32::update(data, tail)?,
            }),
            [1, tail @ ..] => Ok(UpdateTestUpdate::B(bool::update(data, tail)?)),
            _ => Err(PatchError::InvalidPath),
        }
    }
}

fn test(data: &ParamData, path: &[u32]) {
    match UpdateTest::update(data, path) {
        Ok(UpdateTestUpdate::A { 0: a }) => println!("{a}"),
        Ok(UpdateTestUpdate::B(b)) => println!("{b}"),
        _ => {}
    }
}
