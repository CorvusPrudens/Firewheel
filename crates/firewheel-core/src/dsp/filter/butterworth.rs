use super::{
    cascade::{ChainedCascadeUpTo, FilterCascadeUpTo},
    prototype::Prototype,
    spec::{FilterOrder, ResponseType},
};

pub struct Butterworth;

impl Prototype for Butterworth {
    fn design<const ORDER: FilterOrder>(response_type: ResponseType) -> FilterCascadeUpTo<ORDER> {
        todo!()
    }
    fn design_composite<const ORDER: FilterOrder>(
        response_type: ResponseType,
    ) -> ChainedCascadeUpTo<2, ORDER> {
        todo!()
    }
}
