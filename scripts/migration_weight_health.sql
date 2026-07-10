-- Add active_calls to gateway_health_status to track concurrent calls in database
ALTER TABLE gateway_health_status ADD COLUMN IF NOT EXISTS active_calls INTEGER NOT NULL DEFAULT 0;
