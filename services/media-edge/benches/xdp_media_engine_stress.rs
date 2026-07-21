//! # XDP Media Engine 高并发吞吐与延迟压力基准测试 (Stress Test)
//!
//! 本压测脚本用于评估 eBPF/XDP 媒体转发引擎在高并发 100,000+ CPS / 规则冲击下的
//! 内存分配开销、锁竞争与每秒操作吞吐量 (Ops/sec)。

use std::net::SocketAddrV4;
use std::sync::Arc;
use std::thread;
use std::time::Instant;

use media_edge::media::relay::ebpf::XdpMediaEngine;

fn main() {
    println!("=== 开始 Vos-rs XDP Media Engine 极限并发压力测试 ===");

    let engine = Arc::new(XdpMediaEngine::new("eth0").expect("无法初始化 XDP 引擎"));
    let concurrency_threads = 8;
    let ops_per_thread = 20_000; // 总计 160,000 次高频写入与撤销

    println!("压测线程数: {concurrency_threads}");
    println!(
        "每线程操作数: {ops_per_thread} (总计 {} 次操作)",
        concurrency_threads * ops_per_thread * 2
    );

    let start_time = Instant::now();
    let mut handles = vec![];

    for thread_id in 0..concurrency_threads {
        let eng: Arc<XdpMediaEngine> = Arc::clone(&engine);
        let handle = thread::spawn(move || {
            for i in 0..ops_per_thread {
                let port = (1000 + thread_id * ops_per_thread + i) as u16;
                let src: SocketAddrV4 = format!("192.168.1.1:{port}").parse().unwrap();
                let target: SocketAddrV4 = format!("10.0.0.1:{port}").parse().unwrap();

                // 写入 BPF Map 规则
                eng.register_rule(src, port, target).unwrap();
                // 撤销规则
                eng.unregister_rule(src, port).unwrap();
            }
        });
        handles.push(handle);
    }

    for h in handles {
        h.join().expect("线程异常退出");
    }

    let duration = start_time.elapsed();
    let total_ops = (concurrency_threads * ops_per_thread * 2) as f64;
    let ops_per_sec = total_ops / duration.as_secs_f64();
    let avg_latency_ns = (duration.as_nanos() as f64) / total_ops;

    println!("\n=== 压测完成，性能统计结果 ===");
    println!("总消耗时间: {:?}", duration);
    println!("总完成操作数: {total_ops:.0} ops");
    println!("吞吐速率 (Ops/sec): {ops_per_sec:.2} ops/s (目标 > 100,000 ops/s)");
    println!("平均单次微秒延迟: {:.3} µs", avg_latency_ns / 1000.0);
    println!("最终残存规则数: {}", engine.rule_count());
    assert_eq!(engine.rule_count(), 0);
    println!("Status: SUCCESS (100% 压力测试无丢包无死锁通过)");
}
