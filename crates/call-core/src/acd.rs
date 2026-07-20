use std::collections::{HashMap, VecDeque};
use std::time::{Instant, SystemTime};

/// 座席工作会话状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum AgentState {
    Online,
    Busy,
    Idle,
    Offline,
}

/// 座席分配策略
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum AllocationStrategy {
    LongestIdle,
    RoundRobin,
}

/// 排队呼叫队列
#[derive(Debug, Clone)]
pub struct WaitingCall {
    pub call_id: String,
    pub enter_time: Instant,
    pub priority: u32,
}

/// 座席详情
#[derive(Debug, Clone)]
pub struct AgentSession {
    pub agent_id: String,
    pub state: AgentState,
    pub last_state_change: SystemTime,
}

impl AgentSession {
    pub fn new(agent_id: impl Into<String>) -> Self {
        Self {
            agent_id: agent_id.into(),
            state: AgentState::Offline,
            last_state_change: SystemTime::now(),
        }
    }

    pub fn set_state(&mut self, state: AgentState) {
        if self.state != state {
            self.state = state;
            self.last_state_change = SystemTime::now();
        }
    }
}

/// ACD 引擎
pub struct AcdEngine {
    pub queue_id: String,
    pub strategy: AllocationStrategy,
    pub agents: HashMap<String, AgentSession>,
    pub waiting_queue: VecDeque<WaitingCall>,
    pub round_robin_index: usize,
}

impl AcdEngine {
    pub fn new(queue_id: impl Into<String>, strategy: AllocationStrategy) -> Self {
        Self {
            queue_id: queue_id.into(),
            strategy,
            agents: HashMap::new(),
            waiting_queue: VecDeque::new(),
            round_robin_index: 0,
        }
    }

    pub fn add_agent(&mut self, agent: AgentSession) {
        self.agents.insert(agent.agent_id.clone(), agent);
    }

    pub fn update_agent_state(&mut self, agent_id: &str, state: AgentState) {
        if let Some(agent) = self.agents.get_mut(agent_id) {
            agent.set_state(state);
        }
    }

    /// 将呼叫入队，并尝试立即分配
    pub fn enqueue_call(&mut self, call_id: String, priority: u32) -> EnqueueResult {
        let call = WaitingCall {
            call_id: call_id.clone(),
            enter_time: Instant::now(),
            priority,
        };
        self.waiting_queue.push_back(call);

        self.try_allocate()
    }

    /// 尝试从队列中为呼叫分配空闲座席
    pub fn try_allocate(&mut self) -> EnqueueResult {
        if self.waiting_queue.is_empty() {
            return EnqueueResult::NoWaitingCall;
        }

        let mut idle_agents: Vec<&AgentSession> = self
            .agents
            .values()
            .filter(|a| a.state == AgentState::Idle)
            .collect();

        if idle_agents.is_empty() {
            // 没有空闲座席，返回需等待并播放 MoH
            return EnqueueResult::QueuedPlayMoh;
        }

        let chosen_agent_id = match self.strategy {
            AllocationStrategy::LongestIdle => {
                idle_agents.sort_by_key(|a| a.last_state_change);
                match idle_agents.first() {
                    Some(agent) => agent.agent_id.clone(),
                    None => return EnqueueResult::QueuedPlayMoh,
                }
            }
            AllocationStrategy::RoundRobin => {
                idle_agents.sort_by(|a, b| a.agent_id.cmp(&b.agent_id));
                let idx = self.round_robin_index % idle_agents.len();
                self.round_robin_index = (idx + 1) % idle_agents.len();
                idle_agents[idx].agent_id.clone()
            }
        };

        // 从队列中取出一个
        let allocated_call = match self.waiting_queue.pop_front() {
            Some(call) => call,
            None => return EnqueueResult::NoWaitingCall,
        };

        // 将座席设为 Busy
        self.update_agent_state(&chosen_agent_id, AgentState::Busy);

        EnqueueResult::Allocated(allocated_call.call_id, chosen_agent_id)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum EnqueueResult {
    NoWaitingCall,
    QueuedPlayMoh,
    Allocated(String, String), // (call_id, agent_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_longest_idle_strategy() {
        let mut acd = AcdEngine::new("q1", AllocationStrategy::LongestIdle);
        
        let mut a1 = AgentSession::new("agent1");
        let mut a2 = AgentSession::new("agent2");
        
        a1.set_state(AgentState::Idle);
        a1.last_state_change = SystemTime::now() - Duration::from_secs(100); // a1 is much older

        a2.set_state(AgentState::Idle);
        a2.last_state_change = SystemTime::now() - Duration::from_secs(10);

        acd.add_agent(a1);
        acd.add_agent(a2);

        let res = acd.enqueue_call("call1".into(), 0);
        assert_eq!(res, EnqueueResult::Allocated("call1".into(), "agent1".into()));

        let res2 = acd.enqueue_call("call2".into(), 0);
        assert_eq!(res2, EnqueueResult::Allocated("call2".into(), "agent2".into()));

        let res3 = acd.enqueue_call("call3".into(), 0);
        assert_eq!(res3, EnqueueResult::QueuedPlayMoh);
    }

    #[test]
    fn test_round_robin_strategy() {
        let mut acd = AcdEngine::new("q1", AllocationStrategy::RoundRobin);
        
        let mut a1 = AgentSession::new("agent1");
        let mut a2 = AgentSession::new("agent2");
        a1.set_state(AgentState::Idle);
        a2.set_state(AgentState::Idle);
        acd.add_agent(a1);
        acd.add_agent(a2);

        let res1 = acd.enqueue_call("call1".into(), 0);
        let res2 = acd.enqueue_call("call2".into(), 0);

        assert_eq!(res1, EnqueueResult::Allocated("call1".into(), "agent1".into()));
        assert_eq!(res2, EnqueueResult::Allocated("call2".into(), "agent2".into()));
    }

    #[test]
    fn test_agent_deallocation_and_retry_allocate() {
        let mut acd = AcdEngine::new("q1", AllocationStrategy::LongestIdle);
        let mut a1 = AgentSession::new("agent1");
        a1.set_state(AgentState::Busy);
        acd.add_agent(a1);

        // Enqueue when no idle agent
        let res1 = acd.enqueue_call("call1".into(), 1);
        assert_eq!(res1, EnqueueResult::QueuedPlayMoh);

        // Agent becomes Idle
        acd.update_agent_state("agent1", AgentState::Idle);

        // Try allocate again
        let res2 = acd.try_allocate();
        assert_eq!(res2, EnqueueResult::Allocated("call1".into(), "agent1".into()));
    }
}

