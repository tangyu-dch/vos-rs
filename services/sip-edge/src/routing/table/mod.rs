//! 路由表加载与热路径缓存预热模块。

mod helpers;
mod refresh;
mod warmup;

type AnyError = Box<dyn std::error::Error + Send + Sync>;

pub(crate) use helpers::{parse_gateway_target, route_table_from_config};
pub(crate) use refresh::spawn_periodic_route_refresh;
pub(crate) use warmup::warm_hot_path_redis_cache;
