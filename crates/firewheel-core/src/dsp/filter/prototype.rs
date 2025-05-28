use super::{
    cascade::{ChainedCascadeUpTo, FilterCascadeUpTo},
    spec::{FilterOrder, ResponseType},
};

pub trait Prototype {
    fn design<const ORDER: FilterOrder>(response_type: ResponseType) -> FilterCascadeUpTo<ORDER>;
    fn design_composite<const ORDER: FilterOrder>(
        response_type: ResponseType,
    ) -> ChainedCascadeUpTo<2, ORDER>;
}
