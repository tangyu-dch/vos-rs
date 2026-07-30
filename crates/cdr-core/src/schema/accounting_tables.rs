/// Migrates call settlement to support one access charge and one egress cost per call.
pub(crate) const MIGRATE_DUAL_CALL_ACCOUNTING_SQL: &str = r#"
ALTER TABLE billing_ledger
    ADD COLUMN IF NOT EXISTS entry_type TEXT NOT NULL DEFAULT 'call_charge';
ALTER TABLE billing_ledger DROP CONSTRAINT IF EXISTS billing_ledger_call_id_key;
DROP INDEX IF EXISTS billing_ledger_call_id_key;
ALTER TABLE billing_ledger DROP CONSTRAINT IF EXISTS billing_ledger_entry_type_check;
ALTER TABLE billing_ledger
    ADD CONSTRAINT billing_ledger_entry_type_check
    CHECK (entry_type IN ('call_charge', 'call_cost')) NOT VALID;
CREATE UNIQUE INDEX IF NOT EXISTS idx_billing_ledger_call_entry_unique
    ON billing_ledger (call_id, entry_type);
CREATE INDEX IF NOT EXISTS idx_billing_ledger_entry_created
    ON billing_ledger (entry_type, created_at DESC);
"#;

#[cfg(test)]
mod tests {
    use super::MIGRATE_DUAL_CALL_ACCOUNTING_SQL;

    #[test]
    fn dual_accounting_allows_one_entry_per_call_side() {
        assert!(
            MIGRATE_DUAL_CALL_ACCOUNTING_SQL.contains("entry_type IN ('call_charge', 'call_cost')")
        );
        assert!(MIGRATE_DUAL_CALL_ACCOUNTING_SQL.contains("(call_id, entry_type)"));
    }
}
