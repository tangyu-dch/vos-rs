use crate::{
    CallerPool, CallerPoolMember, DidDestination, EgressEndpoint, EgressGroup, EgressGroupMember,
    NumberAllocation, PostgresCdrStore, SourceOutboundPolicy, TrunkIpRule,
};

impl PostgresCdrStore {
    /// Lists IP authentication rules for one ingress trunk.
    pub async fn list_trunk_ip_rules(
        &self,
        trunk_id: &str,
    ) -> Result<Vec<TrunkIpRule>, sqlx::Error> {
        sqlx::query_as(
            "SELECT id, trunk_id, cidr::text AS cidr, source_port, transport, description, enabled \
             FROM trunk_ip_rules WHERE trunk_id=$1 ORDER BY id",
        )
        .bind(trunk_id)
        .fetch_all(&self.pool)
        .await
    }

    /// Replaces all IP authentication rules and rejects overlap with other trunks.
    pub async fn replace_trunk_ip_rules(
        &self,
        trunk_id: &str,
        rules: &[TrunkIpRule],
    ) -> Result<(), sqlx::Error> {
        let mut transaction = self.pool.begin().await?;
        for (index, rule) in rules.iter().enumerate().filter(|(_, rule)| rule.enabled) {
            for other in rules.iter().skip(index + 1).filter(|rule| rule.enabled) {
                let ports_overlap = rule.source_port.is_none()
                    || other.source_port.is_none()
                    || rule.source_port == other.source_port;
                if rule.transport == other.transport && ports_overlap {
                    let overlaps: bool = sqlx::query_scalar("SELECT $1::cidr && $2::cidr")
                        .bind(&rule.cidr)
                        .bind(&other.cidr)
                        .fetch_one(&mut *transaction)
                        .await?;
                    if overlaps {
                        return Err(sqlx::Error::Protocol(format!(
                            "同一批次 IP 规则 {} 与 {} 重叠",
                            rule.cidr, other.cidr
                        )));
                    }
                }
            }
        }
        for rule in rules {
            if !rule.enabled {
                continue;
            }
            let conflict: Option<(String,)> = sqlx::query_as(
                "SELECT trunk_id FROM trunk_ip_rules \
                 WHERE trunk_id <> $1 AND enabled AND cidr && $2::cidr \
                   AND transport=$3 AND (source_port IS NULL OR $4::int IS NULL OR source_port=$4) \
                 LIMIT 1",
            )
            .bind(trunk_id)
            .bind(&rule.cidr)
            .bind(&rule.transport)
            .bind(rule.source_port)
            .fetch_optional(&mut *transaction)
            .await?;
            if let Some((conflicting_trunk,)) = conflict {
                return Err(sqlx::Error::Protocol(format!(
                    "IP 规则 {} 与中继 {} 重叠",
                    rule.cidr, conflicting_trunk
                )));
            }
        }
        sqlx::query("DELETE FROM trunk_ip_rules WHERE trunk_id=$1")
            .bind(trunk_id)
            .execute(&mut *transaction)
            .await?;
        for rule in rules {
            sqlx::query(
                "INSERT INTO trunk_ip_rules (trunk_id,cidr,source_port,transport,description,enabled) \
                 VALUES ($1,$2::cidr,$3,$4,$5,$6)",
            )
            .bind(trunk_id)
            .bind(&rule.cidr)
            .bind(rule.source_port)
            .bind(&rule.transport)
            .bind(&rule.description)
            .bind(rule.enabled)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await
    }

    /// Lists signaling endpoints for one egress trunk.
    pub async fn list_egress_endpoints(
        &self,
        trunk_id: &str,
    ) -> Result<Vec<EgressEndpoint>, sqlx::Error> {
        sqlx::query_as(
            "SELECT id,trunk_id,host,port,transport,priority,enabled FROM egress_endpoints \
             WHERE trunk_id=$1 ORDER BY priority DESC,id",
        )
        .bind(trunk_id)
        .fetch_all(&self.pool)
        .await
    }

    /// Replaces all signaling endpoints for one egress trunk.
    pub async fn replace_egress_endpoints(
        &self,
        trunk_id: &str,
        endpoints: &[EgressEndpoint],
    ) -> Result<(), sqlx::Error> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query("DELETE FROM egress_endpoints WHERE trunk_id=$1")
            .bind(trunk_id)
            .execute(&mut *transaction)
            .await?;
        for endpoint in endpoints {
            sqlx::query(
                "INSERT INTO egress_endpoints (trunk_id,host,port,transport,priority,enabled) \
                 VALUES ($1,$2,$3,$4,$5,$6)",
            )
            .bind(trunk_id)
            .bind(&endpoint.host)
            .bind(endpoint.port)
            .bind(&endpoint.transport)
            .bind(endpoint.priority)
            .bind(endpoint.enabled)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await
    }

    /// Sets the unique physical owner of a number.
    pub async fn set_number_owner(
        &self,
        number: &str,
        owner_egress_trunk_id: &str,
    ) -> Result<bool, sqlx::Error> {
        let result = sqlx::query(
            "UPDATE number_inventory SET owner_egress_trunk_id=$2,gateway_id=$2,updated_at=now() \
             WHERE number=$1 AND EXISTS (SELECT 1 FROM sip_gateways WHERE id=$2 AND role='egress')",
        )
        .bind(number)
        .bind(owner_egress_trunk_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Lists number use grants, optionally scoped to one number.
    pub async fn list_number_allocations(
        &self,
        number: Option<&str>,
    ) -> Result<Vec<NumberAllocation>, sqlx::Error> {
        sqlx::query_as(
            "SELECT id,number,source_type,source_id,enabled FROM number_allocations \
             WHERE ($1::text IS NULL OR number=$1) ORDER BY number,id",
        )
        .bind(number)
        .fetch_all(&self.pool)
        .await
    }

    /// Replaces all use grants for one number.
    pub async fn replace_number_allocations(
        &self,
        number: &str,
        allocations: &[NumberAllocation],
    ) -> Result<(), sqlx::Error> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query("DELETE FROM number_allocations WHERE number=$1")
            .bind(number)
            .execute(&mut *transaction)
            .await?;
        for allocation in allocations {
            sqlx::query(
                "INSERT INTO number_allocations(number,source_type,source_id,enabled) VALUES($1,$2,$3,$4)",
            )
            .bind(number)
            .bind(&allocation.source_type)
            .bind(&allocation.source_id)
            .bind(allocation.enabled)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await
    }

    /// Lists caller pools.
    pub async fn list_caller_pools(&self) -> Result<Vec<CallerPool>, sqlx::Error> {
        sqlx::query_as("SELECT * FROM caller_pools ORDER BY id")
            .fetch_all(&self.pool)
            .await
    }

    /// Creates or updates a caller pool.
    pub async fn upsert_caller_pool(&self, pool: &CallerPool) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO caller_pools(id,owner_source_type,owner_source_id,virtual_alias,strategy,fallback_mode,enabled) \
             VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT(id) DO UPDATE SET \
             owner_source_type=EXCLUDED.owner_source_type,owner_source_id=EXCLUDED.owner_source_id, \
             virtual_alias=EXCLUDED.virtual_alias,strategy=EXCLUDED.strategy, \
             fallback_mode=EXCLUDED.fallback_mode,enabled=EXCLUDED.enabled,updated_at=now()",
        )
        .bind(&pool.id)
        .bind(&pool.owner_source_type)
        .bind(&pool.owner_source_id)
        .bind(&pool.virtual_alias)
        .bind(&pool.strategy)
        .bind(&pool.fallback_mode)
        .bind(pool.enabled)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Deletes a caller pool unless a policy still references it.
    pub async fn delete_caller_pool(&self, id: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM caller_pools WHERE id=$1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Lists members of a caller pool.
    pub async fn list_caller_pool_members(
        &self,
        pool_id: &str,
    ) -> Result<Vec<CallerPoolMember>, sqlx::Error> {
        sqlx::query_as("SELECT id,pool_id,number,priority,weight,max_concurrent,enabled FROM caller_pool_members WHERE pool_id=$1 ORDER BY priority DESC,id")
            .bind(pool_id).fetch_all(&self.pool).await
    }

    /// Replaces members of a caller pool.
    pub async fn replace_caller_pool_members(
        &self,
        pool_id: &str,
        members: &[CallerPoolMember],
    ) -> Result<(), sqlx::Error> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query("DELETE FROM caller_pool_members WHERE pool_id=$1")
            .bind(pool_id)
            .execute(&mut *transaction)
            .await?;
        for member in members {
            sqlx::query("INSERT INTO caller_pool_members(pool_id,number,priority,weight,max_concurrent,enabled) VALUES($1,$2,$3,$4,$5,$6)")
                .bind(pool_id).bind(&member.number).bind(member.priority).bind(member.weight)
                .bind(member.max_concurrent).bind(member.enabled).execute(&mut *transaction).await?;
        }
        transaction.commit().await
    }

    /// Lists egress authorization groups.
    pub async fn list_egress_groups(&self) -> Result<Vec<EgressGroup>, sqlx::Error> {
        sqlx::query_as("SELECT * FROM egress_groups ORDER BY id")
            .fetch_all(&self.pool)
            .await
    }

    /// Creates or updates an egress authorization group.
    pub async fn upsert_egress_group(&self, group: &EgressGroup) -> Result<(), sqlx::Error> {
        sqlx::query("INSERT INTO egress_groups(id,name,enabled) VALUES($1,$2,$3) ON CONFLICT(id) DO UPDATE SET name=EXCLUDED.name,enabled=EXCLUDED.enabled,updated_at=now()")
            .bind(&group.id).bind(&group.name).bind(group.enabled).execute(&self.pool).await?;
        Ok(())
    }

    /// Deletes an egress group unless a policy still references it.
    pub async fn delete_egress_group(&self, id: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM egress_groups WHERE id=$1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Lists members of an egress group.
    pub async fn list_egress_group_members(
        &self,
        group_id: &str,
    ) -> Result<Vec<EgressGroupMember>, sqlx::Error> {
        sqlx::query_as("SELECT id,group_id,egress_trunk_id,destination_prefix,priority,weight,time_start,time_end,enabled FROM egress_group_members WHERE group_id=$1 ORDER BY priority DESC,id")
            .bind(group_id).fetch_all(&self.pool).await
    }

    /// Replaces members of an egress group.
    pub async fn replace_egress_group_members(
        &self,
        group_id: &str,
        members: &[EgressGroupMember],
    ) -> Result<(), sqlx::Error> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query("DELETE FROM egress_group_members WHERE group_id=$1")
            .bind(group_id)
            .execute(&mut *transaction)
            .await?;
        for member in members {
            sqlx::query("INSERT INTO egress_group_members(group_id,egress_trunk_id,destination_prefix,priority,weight,time_start,time_end,enabled) VALUES($1,$2,$3,$4,$5,$6,$7,$8)")
                .bind(group_id).bind(&member.egress_trunk_id).bind(&member.destination_prefix)
                .bind(member.priority).bind(member.weight).bind(&member.time_start).bind(&member.time_end)
                .bind(member.enabled).execute(&mut *transaction).await?;
        }
        transaction.commit().await
    }

    /// Lists source outbound policies.
    pub async fn list_source_outbound_policies(
        &self,
    ) -> Result<Vec<SourceOutboundPolicy>, sqlx::Error> {
        sqlx::query_as("SELECT * FROM source_outbound_policies ORDER BY source_type,source_id")
            .fetch_all(&self.pool)
            .await
    }

    /// Creates or updates a source outbound policy.
    pub async fn upsert_source_outbound_policy(
        &self,
        policy: &SourceOutboundPolicy,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("INSERT INTO source_outbound_policies(source_type,source_id,caller_mode,fixed_number,caller_pool_id,egress_mode,direct_egress_trunk_id,egress_group_id,fallback_mode,enabled) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) ON CONFLICT(source_type,source_id) DO UPDATE SET caller_mode=EXCLUDED.caller_mode,fixed_number=EXCLUDED.fixed_number,caller_pool_id=EXCLUDED.caller_pool_id,egress_mode=EXCLUDED.egress_mode,direct_egress_trunk_id=EXCLUDED.direct_egress_trunk_id,egress_group_id=EXCLUDED.egress_group_id,fallback_mode=EXCLUDED.fallback_mode,enabled=EXCLUDED.enabled,updated_at=now()")
            .bind(&policy.source_type).bind(&policy.source_id).bind(&policy.caller_mode)
            .bind(&policy.fixed_number).bind(&policy.caller_pool_id).bind(&policy.egress_mode)
            .bind(&policy.direct_egress_trunk_id).bind(&policy.egress_group_id)
            .bind(&policy.fallback_mode).bind(policy.enabled).execute(&self.pool).await?;
        Ok(())
    }

    /// Deletes one source outbound policy.
    pub async fn delete_source_outbound_policy(
        &self,
        source_type: &str,
        source_id: &str,
    ) -> Result<bool, sqlx::Error> {
        let result = sqlx::query(
            "DELETE FROM source_outbound_policies WHERE source_type=$1 AND source_id=$2",
        )
        .bind(source_type)
        .bind(source_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Lists DID destinations.
    pub async fn list_did_destinations(&self) -> Result<Vec<DidDestination>, sqlx::Error> {
        sqlx::query_as("SELECT * FROM did_destinations ORDER BY number")
            .fetch_all(&self.pool)
            .await
    }

    /// Creates or updates a DID destination.
    pub async fn upsert_did_destination(
        &self,
        destination: &DidDestination,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("INSERT INTO did_destinations(number,tenant_id,target_type,target_id,enabled) VALUES($1,$2,$3,$4,$5) ON CONFLICT(number) DO UPDATE SET tenant_id=EXCLUDED.tenant_id,target_type=EXCLUDED.target_type,target_id=EXCLUDED.target_id,enabled=EXCLUDED.enabled,updated_at=now()")
            .bind(&destination.number).bind(&destination.tenant_id).bind(&destination.target_type)
            .bind(&destination.target_id).bind(destination.enabled).execute(&self.pool).await?;
        Ok(())
    }

    /// Deletes one DID destination.
    pub async fn delete_did_destination(&self, number: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM did_destinations WHERE number=$1")
            .bind(number)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }
}
