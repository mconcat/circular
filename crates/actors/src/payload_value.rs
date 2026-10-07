//! The existing [port shape, value] payload representation, shared by recorded Source inputs and notify checkpoints.
use crate::{Flow, GroundShape, ProductPayload, ProductValue};
use circular_protocol::port_type::{decode_port_shape, encode_port_shape};
pub fn encode_payload(payload: &ProductPayload) -> ProductValue {
    let flow = crate::port_type_from_flow(&Flow::Stream(payload.shape().as_shape().clone()));
    let shape = encode_port_shape(flow.item()).expect("shape of the accepted payload");
    ProductValue::array([shape, payload.value().clone()])
}

pub fn decode_payload(value: ProductValue) -> Option<ProductPayload> {
    let ProductValue::Array(fields) = value else {
        return None;
    };
    let [shape, value] = <[ProductValue; 2]>::try_from(fields).ok()?;
    let shape = decode_port_shape(shape).ok()?;
    let shape = crate::types::shape_from_port_type(shape);
    Some(ProductPayload::new(
        GroundShape::try_new(shape).ok()?,
        value,
    ))
}
