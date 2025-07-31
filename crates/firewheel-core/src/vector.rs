/// A two-dimensional vector type.
#[derive(Default, Debug, Clone, Copy, PartialEq, PartialOrd)]
#[cfg_attr(feature = "bevy", derive(bevy_ecs::prelude::Component))]
#[cfg_attr(feature = "bevy_reflect", derive(bevy_reflect::Reflect))]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Vec2 {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// A three-dimensional vector type.
#[derive(Default, Debug, Clone, Copy, PartialEq, PartialOrd)]
#[cfg_attr(feature = "bevy", derive(bevy_ecs::prelude::Component))]
#[cfg_attr(feature = "bevy_reflect", derive(bevy_reflect::Reflect))]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }
}

impl<T> From<T> for Vec2
where
    T: Into<[f32; 2]>,
{
    fn from(value: T) -> Self {
        let v: [f32; 2] = value.into();
        Self { x: v[0], y: v[1] }
    }
}

impl<T> From<T> for Vec3
where
    T: Into<[f32; 3]>,
{
    fn from(value: T) -> Self {
        let v: [f32; 3] = value.into();
        Self {
            x: v[0],
            y: v[1],
            z: v[2],
        }
    }
}

#[cfg(feature = "glam")]
impl Into<glam::Vec2> for Vec2 {
    fn into(self) -> glam::Vec2 {
        glam::Vec2::new(self.x, self.y)
    }
}

#[cfg(feature = "glam")]
impl Into<glam::Vec3> for Vec3 {
    fn into(self) -> glam::Vec3 {
        glam::Vec3::new(self.x, self.y, self.z)
    }
}
