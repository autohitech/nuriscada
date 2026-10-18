// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! 주기 스캔 스케줄러, 값 엔진, 지터 계측, 값·알람 허브.
//!
//! 순수 로직(환산, 알람)은 `nuriscada-core`에 있고, 이 크레이트가 드라이버 호출과 결합한다.

pub mod engine;
pub mod hub;
pub mod jitter;
pub mod plan;
pub mod runner;

pub use hub::{AlarmRecord, DeviceStatus, Hub, Quality, TagSnapshot, TagValue, Update, now_ms};
pub use plan::{PlanLimits, PlannedBlock, plan};
pub use runner::run_device;
