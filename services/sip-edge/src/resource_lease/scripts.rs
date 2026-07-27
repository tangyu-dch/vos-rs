pub(super) const RENEWAL_INTERVAL_SECS: u64 = 60;
pub(super) const RENEWAL_TTL_SECS: u64 = 300;

pub(super) const ACQUIRE_SCRIPT: &str = r#"
local redis_time = redis.call('TIME')
local now = tonumber(redis_time[1])
local expires = now + tonumber(ARGV[1])
local call_id = ARGV[2]
local caller_number = ARGV[3]
local gateway_id = ARGV[4]
local number_max_concurrent = tonumber(ARGV[5])
local trunk_max_concurrent = tonumber(ARGV[6])
local lease_value = caller_number .. '\31' .. gateway_id

local expired_calls = redis.call('ZRANGEBYSCORE', KEYS[2], '-inf', now)
for _, expired_call in ipairs(expired_calls) do
  redis.call('HDEL', KEYS[1], expired_call)
end
redis.call('ZREMRANGEBYSCORE', KEYS[2], '-inf', now)

redis.call('ZREMRANGEBYSCORE', KEYS[3], '-inf', now)
redis.call('ZREMRANGEBYSCORE', KEYS[4], '-inf', now)

local existing = redis.call('HGET', KEYS[1], call_id)
if existing then
  if existing ~= lease_value then
    return -3
  end
  local call_expiry = tonumber(redis.call('ZSCORE', KEYS[2], call_id)) or 0
  local number_expiry = caller_number == '' and call_expiry or tonumber(redis.call('ZSCORE', KEYS[3], call_id)) or 0
  local trunk_expiry = tonumber(redis.call('ZSCORE', KEYS[4], call_id)) or 0
  expires = math.max(expires, call_expiry, number_expiry, trunk_expiry)
else
  if caller_number ~= '' and number_max_concurrent > 0 and redis.call('ZCARD', KEYS[3]) >= number_max_concurrent then
    return -1
  end
  if trunk_max_concurrent > 0 and redis.call('ZCARD', KEYS[4]) >= trunk_max_concurrent then
    return -2
  end
end

redis.call('HSET', KEYS[1], call_id, lease_value)
redis.call('ZADD', KEYS[2], expires, call_id)
if caller_number ~= '' then
  redis.call('ZADD', KEYS[3], expires, call_id)
end
redis.call('ZADD', KEYS[4], expires, call_id)
return 1
"#;

pub(super) const RELEASE_SCRIPT: &str = r#"
local call_id = ARGV[1]
local caller_number = ARGV[2]
local gateway_id = ARGV[3]
local lease_value = caller_number .. '\31' .. gateway_id
if redis.call('HGET', KEYS[1], call_id) ~= lease_value then
  return 0
end
redis.call('HDEL', KEYS[1], call_id)
redis.call('ZREM', KEYS[2], call_id)
if caller_number ~= '' then
  redis.call('ZREM', KEYS[3], call_id)
end
redis.call('ZREM', KEYS[4], call_id)
return 1
"#;

pub(super) const RENEW_SCRIPT: &str = r#"
local redis_time = redis.call('TIME')
local now = tonumber(redis_time[1])
local expires = now + tonumber(ARGV[1])
local call_id = ARGV[2]
local caller_number = ARGV[3]
local gateway_id = ARGV[4]
local lease_value = caller_number .. '\31' .. gateway_id

local existing = redis.call('HGET', KEYS[1], call_id)
if not existing then
  return 0
end
if existing ~= lease_value then
  return -3
end

local call_expiry = tonumber(redis.call('ZSCORE', KEYS[2], call_id))
local number_expiry = caller_number == '' and call_expiry or tonumber(redis.call('ZSCORE', KEYS[3], call_id))
local trunk_expiry = tonumber(redis.call('ZSCORE', KEYS[4], call_id))
if not call_expiry or not number_expiry or not trunk_expiry or call_expiry <= now or number_expiry <= now or trunk_expiry <= now then
  redis.call('HDEL', KEYS[1], call_id)
  redis.call('ZREM', KEYS[2], call_id)
  if caller_number ~= '' then
    redis.call('ZREM', KEYS[3], call_id)
  end
  redis.call('ZREM', KEYS[4], call_id)
  return -4
end

local renewal_expiry = math.max(expires, call_expiry, number_expiry, trunk_expiry)
redis.call('ZADD', KEYS[2], renewal_expiry, call_id)
if caller_number ~= '' then
  redis.call('ZADD', KEYS[3], renewal_expiry, call_id)
end
redis.call('ZADD', KEYS[4], renewal_expiry, call_id)
return 1
"#;

pub(super) const CALLS_KEY: &str = "vos_rs:{resource-leases}:calls";
pub(super) const CALL_EXPIRY_KEY: &str = "vos_rs:{resource-leases}:call-expiry";
