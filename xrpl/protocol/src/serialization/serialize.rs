//! Tiny serialization helpers from `xrpl/protocol/serialize.h`.

use basics::str_hex::str_hex;

use crate::{HashPrefix, STObject, Serializer, StBase};

pub fn serialize_blob<T>(object: &T) -> Vec<u8>
where
    T: StBase + ?Sized,
{
    let mut serializer = Serializer::new(0);
    object.add(&mut serializer);
    serializer.data().to_vec()
}

pub fn serialize_prefixed_blob<T>(prefix: HashPrefix, object: &T) -> Vec<u8>
where
    T: StBase + ?Sized,
{
    let mut serializer = Serializer::new(0);
    serializer.add32_prefix(prefix);
    object.add(&mut serializer);
    serializer.data().to_vec()
}

pub fn serialize_hex(object: &STObject) -> String {
    str_hex(serialize_blob(object))
}
