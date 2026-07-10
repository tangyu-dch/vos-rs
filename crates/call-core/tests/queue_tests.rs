use call_core::{Agent, AgentState, CallId, CallQueue, QueueStrategy};
use std::time::Duration;

#[test]
fn test_round_robin_queue_allocation() {
    let mut queue = CallQueue::new("sales", "Sales Queue", QueueStrategy::RoundRobin);
    queue.add_agent(Agent::new("agent1", 10));
    queue.add_agent(Agent::new("agent2", 10));

    // Enqueue 2 calls
    queue.enqueue_call(CallId::new("call1"));
    queue.enqueue_call(CallId::new("call2"));

    // Poll the queue
    let allocations = queue.poll_queue();
    assert_eq!(allocations.len(), 2);

    // Round-robin logic: agent1 and agent2 should get one call each (order-sorted)
    let a1 = &allocations[0];
    let a2 = &allocations[1];
    assert_eq!(a1.0.as_str(), "call1");
    assert_eq!(a2.0.as_str(), "call2");

    // Check states
    assert_eq!(
        queue.agents.get("agent1").unwrap().state,
        AgentState::Ringing
    );
    assert_eq!(
        queue.agents.get("agent2").unwrap().state,
        AgentState::Ringing
    );

    // End call1 on agent1
    queue.record_call_end("agent1", Duration::from_secs(10));
    assert_eq!(queue.agents.get("agent1").unwrap().state, AgentState::Idle);
    assert_eq!(queue.agents.get("agent1").unwrap().total_calls_handled, 1);
    assert_eq!(queue.agents.get("agent1").unwrap().total_talk_time_secs, 10);
}

#[test]
fn test_least_active_queue_allocation() {
    let mut queue = CallQueue::new("support", "Support Queue", QueueStrategy::LeastActive);
    let mut a1 = Agent::new("agent1", 10);
    a1.active_calls = 5;
    let mut a2 = Agent::new("agent2", 10);
    a2.active_calls = 2; // Should be chosen first since it has fewer active calls

    queue.add_agent(a1);
    queue.add_agent(a2);

    queue.enqueue_call(CallId::new("call1"));
    let allocations = queue.poll_queue();
    assert_eq!(allocations.len(), 1);
    assert_eq!(allocations[0].1, "agent2");
}

#[test]
fn test_abandon_queued_call() {
    let mut queue = CallQueue::new("support", "Support Queue", QueueStrategy::RoundRobin);
    queue.enqueue_call(CallId::new("call1"));
    assert_eq!(queue.waiting_calls.len(), 1);

    let abandoned = queue.record_abandon(&CallId::new("call1"));
    assert!(abandoned);
    assert_eq!(queue.waiting_calls.len(), 0);
    assert_eq!(queue.metrics.abandoned_calls, 1);
}
