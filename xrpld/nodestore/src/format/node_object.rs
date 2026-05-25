use basics::base_uint::Uint256;
use basics::blob::Blob;
use basics::counted_object::CountedObject;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u32)]
pub enum NodeObjectType {
    Unknown = 0,
    Ledger = 1,
    AccountNode = 3,
    TransactionNode = 4,
    Dummy = 512,
}

impl TryFrom<u8> for NodeObjectType {
    type Error = ();

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value as u32 {
            0 => Ok(Self::Unknown),
            1 => Ok(Self::Ledger),
            3 => Ok(Self::AccountNode),
            4 => Ok(Self::TransactionNode),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct NodeObject {
    _counted: CountedObject<NodeObject>,
    object_type: NodeObjectType,
    hash: Uint256,
    data: Blob,
}

impl NodeObject {
    pub const KEY_BYTES: usize = 32;

    pub fn new(object_type: NodeObjectType, data: Blob, hash: Uint256) -> Self {
        Self {
            _counted: CountedObject::default(),
            object_type,
            hash,
            data,
        }
    }

    pub fn create_object(object_type: NodeObjectType, data: Blob, hash: Uint256) -> Arc<Self> {
        Arc::new(Self::new(object_type, data, hash))
    }

    pub fn object_type(&self) -> NodeObjectType {
        self.object_type
    }

    pub fn get_type(&self) -> NodeObjectType {
        self.object_type()
    }

    pub fn hash(&self) -> &Uint256 {
        &self.hash
    }

    pub fn get_hash(&self) -> &Uint256 {
        self.hash()
    }

    pub fn data(&self) -> &Blob {
        &self.data
    }

    pub fn get_data(&self) -> &Blob {
        self.data()
    }
}
