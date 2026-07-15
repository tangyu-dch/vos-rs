//! # 高并发无锁原子计费槽模块 (Atomic CAS Billing Bucket)
//!
//! 本模块实现基于 AtomicI64 与 CAS 锁自旋循环的并发计费控制引擎，
//! 适用于 1000+ CPS 带扣费一致性防超扣的电信级软交换场景。

use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

#[derive(Debug)]
pub struct AtomicBillingBucket {
    /// 账户可用余额（微分为单位：1微分为 0.000001 元，完全杜绝浮点数计算精度误差）
    balance_micro: AtomicI64,
    /// 当前通话已划拨预授权（Reservation）的总金额
    reserved_micro: AtomicI64,
    /// 计费槽激活状态，一旦余额归零则置为 false，触发强制话单熔断断开
    is_active: AtomicBool,
}

impl AtomicBillingBucket {
    /// 初始化无锁计费槽，输入初始余额（微分）
    pub fn new(initial_balance: i64) -> Self {
        Self {
            balance_micro: AtomicI64::new(initial_balance),
            reserved_micro: AtomicI64::new(0),
            is_active: AtomicBool::new(initial_balance > 0),
        }
    }

    /// 查询当前可用余额
    pub fn balance(&self) -> i64 {
        self.balance_micro.load(Ordering::Relaxed)
    }

    /// 查询当前已划拨预授权额度
    pub fn reserved(&self) -> i64 {
        self.reserved_micro.load(Ordering::Relaxed)
    }

    /// 查询当前计费槽是否处于激活状态
    pub fn is_active(&self) -> bool {
        self.is_active.load(Ordering::Acquire)
    }

    /// 划拨预授权额度 (Reservation)
    /// 扣减可用余额并计入预授权中，若余额不足以划拨，返回错误并触发熔断
    pub fn reserve(&self, amount: i64) -> Result<(), String> {
        if amount <= 0 {
            return Ok(());
        }
        let mut current = self.balance_micro.load(Ordering::Relaxed);
        loop {
            if current < amount {
                self.is_active.store(false, Ordering::Release);
                return Err("Insufficient balance for reservation".to_string());
            }
            let new_val = current - amount;
            match self.balance_micro.compare_exchange_weak(
                current,
                new_val,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    self.reserved_micro.fetch_add(amount, Ordering::SeqCst);
                    if new_val == 0 {
                        self.is_active.store(false, Ordering::Release);
                    }
                    return Ok(());
                }
                Err(actual) => current = actual,
            }
        }
    }

    /// 释放未消费的预授权额度，退回可用余额中
    pub fn release_reservation(&self, amount: i64) {
        if amount <= 0 {
            return;
        }
        self.reserved_micro.fetch_sub(amount, Ordering::SeqCst);
        self.balance_micro.fetch_add(amount, Ordering::SeqCst);
        // 如果退款后可用余额大于 0，重新激活计费槽
        if self.balance_micro.load(Ordering::Relaxed) > 0 {
            self.is_active.store(true, Ordering::Release);
        }
    }

    /// 并发非阻塞原子扣减可用余额
    /// 用于通话过程中的流式扣费（如每秒或每分钟周期扣减）
    pub fn deduct(&self, amount: i64) -> Result<(), String> {
        if amount <= 0 {
            return Ok(());
        }
        let mut current = self.balance_micro.load(Ordering::Relaxed);
        loop {
            if current < amount {
                self.is_active.store(false, Ordering::Release);
                return Err("Insufficient balance".to_string());
            }
            let new_val = current - amount;
            match self.balance_micro.compare_exchange_weak(
                current,
                new_val,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    if new_val == 0 {
                        self.is_active.store(false, Ordering::Release);
                    }
                    return Ok(());
                }
                Err(actual) => current = actual,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn test_atomic_billing_bucket_basic() {
        let bucket = AtomicBillingBucket::new(1000);
        assert_eq!(bucket.balance(), 1000);
        assert_eq!(bucket.reserved(), 0);
        assert!(bucket.is_active());

        bucket.reserve(200).unwrap();
        assert_eq!(bucket.balance(), 800);
        assert_eq!(bucket.reserved(), 200);

        bucket.deduct(300).unwrap();
        assert_eq!(bucket.balance(), 500);

        bucket.release_reservation(200);
        assert_eq!(bucket.balance(), 700);
        assert_eq!(bucket.reserved(), 0);
    }

    #[test]
    fn test_concurrency_cas_billing() {
        let bucket = Arc::new(AtomicBillingBucket::new(10_000));
        let mut handles = vec![];

        for _ in 0..10 {
            let b = Arc::clone(&bucket);
            let handle = thread::spawn(move || {
                for _ in 0..100 {
                    let _ = b.deduct(10);
                }
            });
            handles.push(handle);
        }

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(bucket.balance(), 0);
        assert!(!bucket.is_active());
        assert!(bucket.deduct(1).is_err());
    }
}
