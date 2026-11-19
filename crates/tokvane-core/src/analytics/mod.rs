use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use chrono::{Datelike, FixedOffset, Local, NaiveDate, TimeZone, Utc};
use rusqlite::{params, params_from_iter, types::Value, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::domain::{
    compute_ewma_tps, compute_iqr_upper_fence, provider_ids, sanitize_session_title,
    speed_basis_for_provider, CurrencyCode, SpeedBasis,
    ModelPricingEntry, ObservedRateLimitWindow, SubscriptionPlan, MAX_VALID_OUTPUT_TPS,
    MIN_VALID_TPS_DURATION_MS, MIN_VALID_TPS_OUTPUT_TOKENS,
};
use crate::error::{CoreError, CoreResult};
use crate::pricing::resolve_pricing;
use crate::storage::repository::{
    load_observed_rate_limits, load_preferences, load_pricing_catalog,
    load_subscription_catalog_templates, load_subscription_plans, load_window_samples_map, DAY_MS,
    HOUR_MS,
};
use crate::storage::Storage;
use crate::subscription::{
    evaluate_subscription_plan, evaluate_subscription_templates_from_list, SubscriptionPlanStatus,
};

mod types;
mod common;
mod overview;
mod subscriptions;
mod trends;
mod monthly;
mod models;
mod speed;
#[cfg(test)]
mod tests;

pub use self::types::*;
pub(crate) use self::common::*;
pub(crate) use self::overview::*;
use self::subscriptions::*;
use self::trends::*;
pub use self::monthly::*;
