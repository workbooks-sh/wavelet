//! Replicate adapters — model paths wavelet needs that aren't on Fal.
//!
//! Two adapters wired here:
//! - Seedance 1 Pro (`bytedance/seedance-1-pro`) — subject-reference
//!   img2vid + txt2vid. Pairs with Seedream 4.5 still gen for the
//!   "frozen subject" fix (wb-s4cw).
//!
//! Auth: `Authorization: Token <token>` from `REPLICATE_API_TOKEN`.
//! Prediction lifecycle is async — POST `/v1/predictions` returns
//! `{id, status: 'starting', urls: {get}}`; poll the `get` URL until
//! `status` flips to `succeeded` or `failed`.

pub mod client;
pub mod grounded_sam;
pub mod lipsync;
pub mod seedance;
pub mod wan_r2v;

pub use client::{ReplicateClient, REPLICATE_TOKEN_ENV};
pub use grounded_sam::{
    ReplicateGroundedSamAdapter, MODEL_GROUNDED_SAM, MODEL_GROUNDED_SAM_VERSION,
    PRICE_PER_CALL_USD as GROUNDED_SAM_PRICE_PER_CALL_USD,
};
pub use lipsync::{
    ReplicateSyncLipSyncAdapter, MODEL_SYNC_LIPSYNC_2_PRO, MODEL_SYNC_LIPSYNC_2_PRO_VERSION,
    PRICE_PER_MINUTE_USD as LIPSYNC_PRICE_PER_MINUTE_USD,
};
pub use seedance::{
    ReplicateSeedanceProAdapter, MODEL_SEEDANCE_1_PRO, MODEL_SEEDANCE_1_PRO_VERSION,
    PRICE_PER_SECOND_USD,
};
pub use wan_r2v::{
    ReplicateWanR2vAdapter, MODEL_WAN_2_7_R2V, MODEL_WAN_2_7_R2V_VERSION,
    PRICE_PER_SECOND_USD as WAN_R2V_PRICE_PER_SECOND_USD,
};
