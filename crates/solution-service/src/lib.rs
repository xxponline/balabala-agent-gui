//! `solution-service` — Solution/Project 域的总线服务（change solution-service,
//! design D1/D2）：加载/解析 `balabala.solutions.json`、配置语义（注册门控 /
//! git 口子）、向资产侧交出 Project 描述树；**对 module 与资产零感知**。
//!
//! 形态对齐 `agent-service`：`contract`（线合同）+ `service`（Service impl）+
//! `valid_checker`（幂等工具，View/Server 共用）+ `loader`（解析内核，
//! design D8 同步入口）+ `git`（系统 git 统一出口, change
//! solution-git-worktree design D6）。

pub mod contract;
pub mod git;
pub mod loader;
pub mod service;
pub mod valid_checker;

#[cfg(test)]
mod testkit;

pub use contract::{
    ActionDesc, ActionInvoke, ExecutedBy, GitOpFinished, LoadError, OpError, ProjectDesc,
    ProjectNode, ProjectTree, SolutionItem, SolutionNode, TreeSnapshot, WorktreeNode,
    ACTION_CONFIGURE_GIT, ACTION_CONFIGURE_REGISTER, ACTION_CREATE, ACTION_CREATE_WORKTREE,
    ACTION_DELETE_WORKTREE, ACTION_PULL, ACTION_PUSH, KIND_GIT_OP_FINISHED,
    SOLUTION_NODE_PREFIX, configure_action_args, parse_solution_node, project_actions,
    solution_actions, solution_node_id, worktree_actions,
};
/// 幂等工具 crate 根导出（design D6: View/Server 共用同一实现）。
pub use valid_checker::{configure_is_noop, create_is_noop};
