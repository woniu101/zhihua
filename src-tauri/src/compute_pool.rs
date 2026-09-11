use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ComputePoolMode {
    Single,
    Elastic,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputePoolPolicy {
    pub mode: ComputePoolMode,
    pub max_workers: u32,
    pub max_instances: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputePoolPlanInput {
    pub task_count: u32,
    pub policy: ComputePoolPolicy,
    pub ready_workers: u32,
    pub active_instances: u32,
    pub account_instance_quota_remaining: u32,
    pub capacity_available_instances: u32,
    pub affordable_new_instances: u32,
    #[serde(default = "one")]
    pub workers_per_new_instance: u32,
    pub hourly_cost_minor_per_new_instance: Option<u64>,
    pub estimated_runtime_seconds: Option<u32>,
    pub currency: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ComputePoolPlan {
    pub task_count: u32,
    pub planned_workers: u32,
    pub reusable_workers: u32,
    pub instances_to_create: u32,
    pub queued_tasks: u32,
    pub estimated_new_instance_cost_minor: Option<u64>,
    pub currency: Option<String>,
    pub requires_confirmation: bool,
    pub limiting_factors: Vec<String>,
}

pub fn plan_compute_pool(input: ComputePoolPlanInput) -> ComputePoolPlan {
    if input.task_count == 0 {
        return ComputePoolPlan {
            task_count: 0,
            planned_workers: 0,
            reusable_workers: 0,
            instances_to_create: 0,
            queued_tasks: 0,
            estimated_new_instance_cost_minor: Some(0)
                .filter(|_| input.hourly_cost_minor_per_new_instance.is_some()),
            currency: input.currency,
            requires_confirmation: false,
            limiting_factors: Vec::new(),
        };
    }

    let worker_limit = match input.policy.mode {
        ComputePoolMode::Single => 1,
        ComputePoolMode::Elastic => input.policy.max_workers.max(1),
    };
    let target_workers = input.task_count.min(worker_limit);
    let reusable_workers = input.ready_workers.min(target_workers);
    let missing_workers = target_workers.saturating_sub(reusable_workers);
    let workers_per_instance = input.workers_per_new_instance.max(1);
    let instances_needed = missing_workers.div_ceil(workers_per_instance);
    let policy_instance_slots = input
        .policy
        .max_instances
        .saturating_sub(input.active_instances);
    let instances_to_create = instances_needed
        .min(policy_instance_slots)
        .min(input.account_instance_quota_remaining)
        .min(input.capacity_available_instances)
        .min(input.affordable_new_instances);
    let created_workers = instances_to_create
        .saturating_mul(workers_per_instance)
        .min(missing_workers);
    let planned_workers = reusable_workers.saturating_add(created_workers);
    let queued_tasks = input.task_count.saturating_sub(planned_workers);

    let mut limiting_factors = Vec::new();
    if input.task_count > worker_limit {
        limiting_factors.push("userWorkerLimit".to_owned());
    }
    if instances_needed > policy_instance_slots {
        limiting_factors.push("userInstanceLimit".to_owned());
    }
    if instances_needed > input.account_instance_quota_remaining {
        limiting_factors.push("accountQuota".to_owned());
    }
    if instances_needed > input.capacity_available_instances {
        limiting_factors.push("regionalCapacity".to_owned());
    }
    if instances_needed > input.affordable_new_instances {
        limiting_factors.push("balanceGuard".to_owned());
    }
    limiting_factors.sort();
    limiting_factors.dedup();

    let estimated_new_instance_cost_minor = match (
        input.hourly_cost_minor_per_new_instance,
        input.estimated_runtime_seconds,
    ) {
        (Some(hourly), Some(seconds)) => Some(
            hourly
                .saturating_mul(instances_to_create as u64)
                .saturating_mul(seconds as u64)
                .div_ceil(3600),
        ),
        _ => None,
    };

    ComputePoolPlan {
        task_count: input.task_count,
        planned_workers,
        reusable_workers,
        instances_to_create,
        queued_tasks,
        estimated_new_instance_cost_minor,
        currency: input.currency,
        requires_confirmation: instances_to_create > 0,
        limiting_factors,
    }
}

const fn one() -> u32 {
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn elastic_input() -> ComputePoolPlanInput {
        ComputePoolPlanInput {
            task_count: 6,
            policy: ComputePoolPolicy {
                mode: ComputePoolMode::Elastic,
                max_workers: 8,
                max_instances: 8,
            },
            ready_workers: 1,
            active_instances: 1,
            account_instance_quota_remaining: 7,
            capacity_available_instances: 4,
            affordable_new_instances: 3,
            workers_per_new_instance: 1,
            hourly_cost_minor_per_new_instance: Some(300),
            estimated_runtime_seconds: Some(300),
            currency: Some("CNY".into()),
        }
    }

    #[test]
    fn single_mode_never_plans_more_than_one_worker() {
        let mut input = elastic_input();
        input.policy.mode = ComputePoolMode::Single;
        let plan = plan_compute_pool(input);
        assert_eq!(plan.planned_workers, 1);
        assert_eq!(plan.instances_to_create, 0);
        assert_eq!(plan.queued_tasks, 5);
    }

    #[test]
    fn elastic_mode_uses_the_tightest_live_constraint() {
        let plan = plan_compute_pool(elastic_input());
        assert_eq!(plan.reusable_workers, 1);
        assert_eq!(plan.instances_to_create, 3);
        assert_eq!(plan.planned_workers, 4);
        assert_eq!(plan.queued_tasks, 2);
        assert!(plan.limiting_factors.contains(&"balanceGuard".to_owned()));
        assert_eq!(plan.estimated_new_instance_cost_minor, Some(75));
        assert!(plan.requires_confirmation);
    }

    #[test]
    fn zero_capacity_keeps_tasks_queued_without_requesting_creation() {
        let mut input = elastic_input();
        input.ready_workers = 0;
        input.active_instances = 0;
        input.capacity_available_instances = 0;
        let plan = plan_compute_pool(input);
        assert_eq!(plan.planned_workers, 0);
        assert_eq!(plan.instances_to_create, 0);
        assert_eq!(plan.queued_tasks, 6);
        assert!(!plan.requires_confirmation);
        assert!(plan
            .limiting_factors
            .contains(&"regionalCapacity".to_owned()));
    }
}
