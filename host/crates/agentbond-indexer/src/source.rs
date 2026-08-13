use async_trait::async_trait;
use futures::stream::BoxStream;

use agentbond_db::{
    Commitment, DecodedProjection, RawAccountVersion, RawProtocolEvent, SlotUpdate,
};

use crate::error::IndexerError;

#[derive(Clone, Debug)]
pub enum ChainUpdate {
    Slot(SlotUpdate),
    Account(Box<AccountUpdate>),
    Events(Vec<RawProtocolEvent>),
}

#[derive(Clone, Debug)]
pub struct AccountUpdate {
    pub raw: RawAccountVersion,
    pub projection: Option<DecodedProjection>,
}

#[async_trait]
pub trait ChainSource: Send + Sync {
    async fn subscribe(
        &self,
    ) -> Result<BoxStream<'static, Result<ChainUpdate, IndexerError>>, IndexerError>;
}

pub fn parse_commitment(s: &str) -> Commitment {
    match s {
        "finalized" => Commitment::Finalized,
        "confirmed" => Commitment::Confirmed,
        "dead" => Commitment::Dead,
        _ => Commitment::Processed,
    }
}
