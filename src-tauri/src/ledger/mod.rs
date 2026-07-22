pub(crate) mod commands;
pub(crate) mod domain;
pub(crate) mod service;
pub(crate) mod smoke;
pub(crate) mod sqlite;

use domain::{
    IntegrityReport, LedgerError, LedgerSnapshot, MutationReceipt, SystemClock, UuidIdGenerator,
    WeeklyFacts,
};
use service::TaskService;
use sqlite::SqliteLedgerStore;
use std::{path::Path, sync::Mutex};

type DesktopTaskService = TaskService<SqliteLedgerStore, SystemClock, UuidIdGenerator>;

pub struct LedgerState {
    service: Mutex<DesktopTaskService>,
}

impl LedgerState {
    pub fn open(path: &Path) -> Result<Self, LedgerError> {
        let store = SqliteLedgerStore::open(path)?;
        Ok(Self {
            service: Mutex::new(TaskService::new(store, SystemClock, UuidIdGenerator)),
        })
    }

    pub fn in_memory() -> Result<Self, LedgerError> {
        let store = SqliteLedgerStore::open_in_memory()?;
        Ok(Self {
            service: Mutex::new(TaskService::new(store, SystemClock, UuidIdGenerator)),
        })
    }

    fn with_service<T>(
        &self,
        action: impl FnOnce(&mut DesktopTaskService) -> Result<T, LedgerError>,
    ) -> Result<T, LedgerError> {
        let mut service = self
            .service
            .lock()
            .map_err(|_| LedgerError::storage("账本服务互斥锁已损坏"))?;
        action(&mut service)
    }

    pub fn capture_task(
        &self,
        operation_id: &str,
        title: &str,
    ) -> Result<MutationReceipt, LedgerError> {
        self.with_service(|service| service.capture_task(operation_id, title))
    }

    pub fn complete_task(
        &self,
        operation_id: &str,
        task_id: &str,
    ) -> Result<MutationReceipt, LedgerError> {
        self.with_service(|service| service.complete_task(operation_id, task_id))
    }

    pub fn update_task_title(
        &self,
        operation_id: &str,
        task_id: &str,
        title: &str,
    ) -> Result<MutationReceipt, LedgerError> {
        self.with_service(|service| service.update_task_title(operation_id, task_id, title))
    }

    pub fn update_task_deadline(
        &self,
        operation_id: &str,
        task_id: &str,
        deadline_on: Option<&str>,
    ) -> Result<MutationReceipt, LedgerError> {
        self.with_service(|service| {
            service.update_task_deadline(operation_id, task_id, deadline_on)
        })
    }

    pub fn delete_task(
        &self,
        operation_id: &str,
        task_id: &str,
    ) -> Result<MutationReceipt, LedgerError> {
        self.with_service(|service| service.delete_task(operation_id, task_id))
    }

    pub fn reorder_tasks(
        &self,
        operation_id: &str,
        moved_task_id: &str,
        expected_task_ids: &[String],
        ordered_task_ids: &[String],
    ) -> Result<MutationReceipt, LedgerError> {
        self.with_service(|service| {
            service.reorder_tasks(
                operation_id,
                moved_task_id,
                expected_task_ids,
                ordered_task_ids,
            )
        })
    }

    pub fn undo_completion(
        &self,
        operation_id: &str,
        completion_event_id: &str,
    ) -> Result<MutationReceipt, LedgerError> {
        self.with_service(|service| service.undo_completion(operation_id, completion_event_id))
    }

    pub fn snapshot(&self) -> Result<LedgerSnapshot, LedgerError> {
        self.with_service(|service| service.snapshot())
    }

    pub fn weekly_facts(&self, from_ms: i64, to_ms: i64) -> Result<WeeklyFacts, LedgerError> {
        self.with_service(|service| service.weekly_facts(from_ms, to_ms))
    }

    pub fn verify_integrity(&self) -> Result<IntegrityReport, LedgerError> {
        self.with_service(|service| service.verify_integrity())
    }
}
