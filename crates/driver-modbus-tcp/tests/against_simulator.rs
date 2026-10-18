// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! 시뮬레이터와 연결한 통합 시험.

use std::time::Duration;

use nuriscada_driver_api::{BlockData, DataKind, Driver, DriverError, LinkState, ReadBlock};
use nuriscada_driver_modbus_tcp::{
    ModbusTcpClient, SPACE_COILS, SPACE_DISCRETE_INPUTS, SPACE_HOLDING_REGISTERS,
    SPACE_INPUT_REGISTERS,
};
use nuriscada_sim_modbus::Simulator;

fn block(space: u16, start: u16, count: u16, kind: DataKind) -> ReadBlock {
    ReadBlock {
        unit: 1,
        space,
        start,
        count,
        kind,
    }
}

async fn setup() -> (Simulator, ModbusTcpClient) {
    let sim = Simulator::start("127.0.0.1:0", 200).await.unwrap();
    let client = ModbusTcpClient::new(sim.addr().to_string(), Duration::from_millis(300));
    (sim, client)
}

#[tokio::test]
async fn reads_all_four_areas() {
    let (sim, mut c) = setup().await;
    sim.set_holding(10, &[100, 200, 300]);
    sim.set_input(0, &[7]);
    sim.set_coils(3, &[true, false, true]);
    sim.set_discrete(1, &[true]);

    let h = c
        .read(&block(SPACE_HOLDING_REGISTERS, 10, 3, DataKind::Word))
        .await
        .unwrap();
    assert_eq!(h, BlockData::Words(vec![100, 200, 300]));
    let i = c
        .read(&block(SPACE_INPUT_REGISTERS, 0, 1, DataKind::Word))
        .await
        .unwrap();
    assert_eq!(i, BlockData::Words(vec![7]));
    let coils = c
        .read(&block(SPACE_COILS, 2, 5, DataKind::Bit))
        .await
        .unwrap();
    assert_eq!(
        coils,
        BlockData::Bits(vec![false, true, false, true, false])
    );
    let d = c
        .read(&block(SPACE_DISCRETE_INPUTS, 0, 3, DataKind::Bit))
        .await
        .unwrap();
    assert_eq!(d, BlockData::Bits(vec![false, true, false]));

    let status = c.status();
    assert_eq!(status.state, LinkState::Connected);
    assert_eq!(status.requests_ok, 4);
    assert_eq!(status.requests_failed, 0);
    assert!(status.last_round_trip.is_some());
}

#[tokio::test]
async fn reads_values_that_change() {
    let (sim, mut c) = setup().await;
    let b = block(SPACE_HOLDING_REGISTERS, 0, 1, DataKind::Word);
    sim.set_holding(0, &[1]);
    assert_eq!(c.read(&b).await.unwrap(), BlockData::Words(vec![1]));
    sim.set_holding(0, &[2]);
    assert_eq!(c.read(&b).await.unwrap(), BlockData::Words(vec![2]));
}

#[tokio::test]
async fn device_error_keeps_the_connection() {
    let (_sim, mut c) = setup().await;
    let err = c
        .read(&block(SPACE_HOLDING_REGISTERS, 5000, 1, DataKind::Word))
        .await
        .unwrap_err();
    assert!(matches!(err, DriverError::Device { code: 2, .. }), "{err}");
    assert_eq!(c.status().state, LinkState::Connected);
    assert_eq!(c.status().requests_failed, 1);
    // 이어서 정상 읽기가 된다.
    assert!(
        c.read(&block(SPACE_HOLDING_REGISTERS, 0, 1, DataKind::Word))
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn invalid_requests_are_rejected_locally() {
    let (sim, mut c) = setup().await;
    let before = sim.state().requests_served;
    let unknown = c.read(&block(9, 0, 1, DataKind::Word)).await.unwrap_err();
    assert!(matches!(unknown, DriverError::InvalidRequest(_)));
    let mismatch = c
        .read(&block(SPACE_COILS, 0, 1, DataKind::Word))
        .await
        .unwrap_err();
    assert!(matches!(mismatch, DriverError::InvalidRequest(_)));
    let too_many = c
        .read(&block(SPACE_HOLDING_REGISTERS, 0, 126, DataKind::Word))
        .await
        .unwrap_err();
    assert!(matches!(too_many, DriverError::InvalidRequest(_)));
    // 장치에는 아무것도 보내지 않았다.
    assert_eq!(sim.state().requests_served, before);
}

#[tokio::test]
async fn slow_device_times_out_then_recovers() {
    let (sim, mut c) = setup().await;
    let b = block(SPACE_HOLDING_REGISTERS, 0, 1, DataKind::Word);
    sim.state().response_delay = Duration::from_millis(800);
    assert_eq!(c.read(&b).await.unwrap_err(), DriverError::Timeout);
    assert_eq!(c.status().state, LinkState::Disconnected);

    sim.state().response_delay = Duration::ZERO;
    assert!(c.read(&b).await.is_ok(), "다시 연결해 읽어야 한다");
    assert_eq!(c.status().state, LinkState::Connected);
}

#[tokio::test]
async fn dropped_connection_is_reported_and_recovered() {
    let (sim, mut c) = setup().await;
    let b = block(SPACE_HOLDING_REGISTERS, 0, 1, DataKind::Word);
    assert!(c.read(&b).await.is_ok());

    sim.state().drop_connections = true;
    let err = c.read(&b).await.unwrap_err();
    assert!(matches!(err, DriverError::Connection(_)), "{err}");
    assert_eq!(c.status().state, LinkState::Disconnected);

    sim.state().drop_connections = false;
    assert!(c.read(&b).await.is_ok());
}

#[tokio::test]
async fn connect_failure_is_a_connection_error() {
    // 사용 중이 아닌 포트를 얻는다: 바인딩했다가 바로 닫는다.
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let mut c = ModbusTcpClient::new(format!("127.0.0.1:{port}"), Duration::from_millis(300));
    let err = c.connect().await.unwrap_err();
    assert!(
        matches!(err, DriverError::Connection(_) | DriverError::Timeout),
        "{err}"
    );
    assert_eq!(c.status().state, LinkState::Disconnected);
}
