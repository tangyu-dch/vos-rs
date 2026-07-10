use crate::CallId;
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant, SystemTime};

/// States that an agent can be in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum AgentState {
    Offline,
    Idle,
    Ringing,
    Talking,
    Paused,
}

/// Routing strategies for allocating calls to agents in a queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum QueueStrategy {
    RoundRobin,
    LeastActive,
    Random,
    Weighted,
}

/// Representation of a call center agent.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Agent {
    pub id: String,
    pub state: AgentState,
    pub weight: u32,
    pub active_calls: u32,
    pub last_state_change: SystemTime,
    pub total_calls_handled: u32,
    pub total_talk_time_secs: u64,
}

impl Agent {
    pub fn new(id: impl Into<String>, weight: u32) -> Self {
        Self {
            id: id.into(),
            state: AgentState::Idle,
            weight,
            active_calls: 0,
            last_state_change: SystemTime::now(),
            total_calls_handled: 0,
            total_talk_time_secs: 0,
        }
    }
}

/// Metrics describing the performance of an ACD queue.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct QueueMetrics {
    pub total_calls: u32,
    pub answered_calls: u32,
    pub abandoned_calls: u32,
    pub total_wait_time_secs: u64,
    pub total_talk_time_secs: u64,
}

impl QueueMetrics {
    /// Average Speed of Answer (ASA) in seconds
    pub fn average_speed_of_answer(&self) -> f64 {
        if self.answered_calls == 0 {
            0.0
        } else {
            self.total_wait_time_secs as f64 / self.answered_calls as f64
        }
    }

    /// Average Call Duration (ACD) in seconds
    pub fn average_call_duration(&self) -> f64 {
        if self.answered_calls == 0 {
            0.0
        } else {
            self.total_talk_time_secs as f64 / self.answered_calls as f64
        }
    }
}

/// An active call waiting in the queue.
#[derive(Debug, Clone)]
pub struct QueuedCall {
    pub call_id: CallId,
    pub entered_at: Instant,
}

/// Represents an ACD Call Queue.
#[derive(Debug, Clone)]
pub struct CallQueue {
    pub id: String,
    pub name: String,
    pub strategy: QueueStrategy,
    pub agents: HashMap<String, Agent>,
    pub waiting_calls: VecDeque<QueuedCall>,
    pub metrics: QueueMetrics,
    pub round_robin_index: usize,
}

impl CallQueue {
    pub fn new(id: impl Into<String>, name: impl Into<String>, strategy: QueueStrategy) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            strategy,
            agents: HashMap::new(),
            waiting_calls: VecDeque::new(),
            metrics: QueueMetrics::default(),
            round_robin_index: 0,
        }
    }

    pub fn add_agent(&mut self, agent: Agent) {
        self.agents.insert(agent.id.clone(), agent);
    }

    pub fn remove_agent(&mut self, agent_id: &str) -> Option<Agent> {
        self.agents.remove(agent_id)
    }

    pub fn set_agent_state(&mut self, agent_id: &str, state: AgentState) {
        if let Some(agent) = self.agents.get_mut(agent_id) {
            agent.state = state;
            agent.last_state_change = SystemTime::now();
        }
    }

    /// Enqueues a call and immediately attempts dispatch if agents are idle.
    pub fn enqueue_call(&mut self, call_id: CallId) {
        self.metrics.total_calls += 1;
        self.waiting_calls.push_back(QueuedCall {
            call_id,
            entered_at: Instant::now(),
        });
    }

    /// Selects the best idle agent based on the queue strategy.
    pub fn select_agent(&mut self) -> Option<String> {
        let idle_agent_ids: Vec<String> = self
            .agents
            .iter()
            .filter(|(_, a)| a.state == AgentState::Idle)
            .map(|(id, _)| id.clone())
            .collect();

        if idle_agent_ids.is_empty() {
            return None;
        }

        match self.strategy {
            QueueStrategy::Random => {
                let idx = (Instant::now().elapsed().as_nanos() as usize) % idle_agent_ids.len();
                Some(idle_agent_ids[idx].clone())
            }
            QueueStrategy::LeastActive => {
                // Find agent with lowest active_calls or longest idle time
                let mut best_agent = &idle_agent_ids[0];
                let mut min_calls = self.agents.get(best_agent).unwrap().active_calls;
                for id in &idle_agent_ids {
                    if let Some(agent) = self.agents.get(id) {
                        if agent.active_calls < min_calls {
                            min_calls = agent.active_calls;
                            best_agent = id;
                        }
                    }
                }
                Some(best_agent.clone())
            }
            QueueStrategy::Weighted => {
                // Select agent with highest weight
                let mut best_agent = &idle_agent_ids[0];
                let mut max_weight = self.agents.get(best_agent).unwrap().weight;
                for id in &idle_agent_ids {
                    if let Some(agent) = self.agents.get(id) {
                        if agent.weight > max_weight {
                            max_weight = agent.weight;
                            best_agent = id;
                        }
                    }
                }
                Some(best_agent.clone())
            }
            QueueStrategy::RoundRobin => {
                // Order agents stably by ID to apply round robin index
                let mut sorted_idle = idle_agent_ids;
                sorted_idle.sort();

                let idx = self.round_robin_index % sorted_idle.len();
                let chosen = sorted_idle[idx].clone();
                self.round_robin_index = (idx + 1) % sorted_idle.len();
                Some(chosen)
            }
        }
    }

    /// Polls the queue to allocate waiting calls to idle agents.
    /// Returns a list of allocations: (CallId, AgentId).
    pub fn poll_queue(&mut self) -> Vec<(CallId, String)> {
        let mut allocations = Vec::new();
        while !self.waiting_calls.is_empty() {
            if let Some(agent_id) = self.select_agent() {
                if let Some(queued) = self.waiting_calls.pop_front() {
                    let wait_secs = queued.entered_at.elapsed().as_secs();
                    self.metrics.answered_calls += 1;
                    self.metrics.total_wait_time_secs += wait_secs;

                    // Update agent state
                    if let Some(agent) = self.agents.get_mut(&agent_id) {
                        agent.state = AgentState::Ringing;
                        agent.active_calls += 1;
                        agent.last_state_change = SystemTime::now();
                    }

                    allocations.push((queued.call_id, agent_id));
                }
            } else {
                break; // No idle agents available
            }
        }
        allocations
    }

    /// Records call termination to settle metrics.
    pub fn record_call_end(&mut self, agent_id: &str, talk_duration: Duration) {
        if let Some(agent) = self.agents.get_mut(agent_id) {
            agent.state = AgentState::Idle;
            agent.active_calls = agent.active_calls.saturating_sub(1);
            agent.total_calls_handled += 1;
            agent.total_talk_time_secs += talk_duration.as_secs();
            agent.last_state_change = SystemTime::now();

            self.metrics.total_talk_time_secs += talk_duration.as_secs();
        }
    }

    /// Records call abandon.
    pub fn record_abandon(&mut self, call_id: &CallId) -> bool {
        let mut found = false;
        self.waiting_calls.retain(|c| {
            if &c.call_id == call_id {
                let wait_secs = c.entered_at.elapsed().as_secs();
                self.metrics.abandoned_calls += 1;
                self.metrics.total_wait_time_secs += wait_secs;
                found = true;
                false
            } else {
                true
            }
        });
        found
    }
}
