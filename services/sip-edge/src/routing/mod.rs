//! 路由模块：路由表加载与号码路由。

mod number;
mod table;

pub(crate) use number::{reload_number_routes, spawn_number_route_refresh};
pub(crate) use table::{
    parse_gateway_target, route_table_from_config, spawn_periodic_route_refresh,
    warm_hot_path_redis_cache,
};
