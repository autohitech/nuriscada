// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! 시뮬레이터를 상대로 한 드라이버 통합 시험.

use std::time::Duration;

use nuriscada_driver_api::{BlockData, DataKind, Driver, DriverError, LinkState, ReadBlock};
use nuriscada_driver_s7::{S7Client, SPACE_S7DB, SPACE_S7M};
use nuriscada_sim_s7::Simulator;

fn block(space: u16, unit: u8, start: u16, count: u16, kind: DataKind) -> ReadBlock {
    ReadBlock {
        unit,
        space,
        start,
        count,
        kind,
    }
}

async fn setup() -> (Simulator, S7Client) {
    let sim = Simulator::start("127.0.0.1:0").await.unwrap();
    let client = S7Client::new(sim.addr().to_string(), Duration::from_secs(2));
    (sim, client)
}

#[tokio::test]
async fn db_워드를_읽는다() {
    let (sim, mut client) = setup().await;
    sim.set_db(1, 0, &[0x1234, 0x5678]);
    assert_eq!(
        client
            .read(&block(SPACE_S7DB, 1, 0, 2, DataKind::Word))
            .await
            .unwrap(),
        BlockData::Words(vec![0x1234, 0x5678])
    );
    assert_eq!(client.status().state, LinkState::Connected);
}

#[tokio::test]
async fn m_비트를_읽는다() {
    let (sim, mut client) = setup().await;
    // M80.0, M80.2: 비트 번호 640, 642.
    sim.set_m(640, &[true, false, true]);
    assert_eq!(
        client
            .read(&block(SPACE_S7M, 0, 640, 3, DataKind::Bit))
            .await
            .unwrap(),
        BlockData::Bits(vec![true, false, true])
    );
}

#[tokio::test]
async fn 바뀌는_값을_읽는다() {
    let (sim, mut client) = setup().await;
    sim.set_db(1, 10, &[1]);
    assert_eq!(
        client
            .read(&block(SPACE_S7DB, 1, 10, 1, DataKind::Word))
            .await
            .unwrap(),
        BlockData::Words(vec![1])
    );
    sim.set_db(1, 10, &[2]);
    assert_eq!(
        client
            .read(&block(SPACE_S7DB, 1, 10, 1, DataKind::Word))
            .await
            .unwrap(),
        BlockData::Words(vec![2])
    );
}

#[tokio::test]
async fn 장치_오류는_연결을_유지한다() {
    let (_sim, mut client) = setup().await;
    // 없는 DB 9.
    let err = client
        .read(&block(SPACE_S7DB, 9, 0, 1, DataKind::Word))
        .await
        .unwrap_err();
    assert!(matches!(err, DriverError::Device { .. }));
    assert_eq!(client.status().state, LinkState::Connected);
}

#[tokio::test]
async fn 잘못된_요청은_미리_거부한다() {
    let (_sim, mut client) = setup().await;
    assert!(matches!(
        client.read(&block(999, 0, 0, 1, DataKind::Word)).await,
        Err(DriverError::InvalidRequest(_))
    ));
    assert!(matches!(
        client
            .read(&block(SPACE_S7DB, 1, 0, 1, DataKind::Bit))
            .await,
        Err(DriverError::InvalidRequest(_))
    ));
    assert!(matches!(
        client
            .read(&block(SPACE_S7DB, 1, 0, 0, DataKind::Word))
            .await,
        Err(DriverError::InvalidRequest(_))
    ));
}

#[tokio::test]
async fn 느린_장치는_시간_초과_후_회복한다() {
    let (sim, mut client) = setup().await;
    sim.set_db(1, 0, &[7]);
    sim.state().response_delay = Duration::from_secs(5);
    assert!(matches!(
        client
            .read(&block(SPACE_S7DB, 1, 0, 1, DataKind::Word))
            .await,
        Err(DriverError::Timeout)
    ));
    sim.state().response_delay = Duration::ZERO;
    assert_eq!(
        client
            .read(&block(SPACE_S7DB, 1, 0, 1, DataKind::Word))
            .await
            .unwrap(),
        BlockData::Words(vec![7])
    );
}

#[tokio::test]
async fn 끊긴_연결은_보고하고_회복한다() {
    let (sim, mut client) = setup().await;
    sim.set_db(1, 0, &[7]);
    sim.state().drop_connections = true;
    assert!(matches!(
        client
            .read(&block(SPACE_S7DB, 1, 0, 1, DataKind::Word))
            .await,
        Err(DriverError::Connection(_))
    ));
    sim.state().drop_connections = false;
    assert_eq!(
        client
            .read(&block(SPACE_S7DB, 1, 0, 1, DataKind::Word))
            .await
            .unwrap(),
        BlockData::Words(vec![7])
    );
}

#[tokio::test]
async fn 연결_실패는_연결_오류다() {
    let mut client = S7Client::new("127.0.0.1:1", Duration::from_millis(300));
    assert!(matches!(
        client.connect().await,
        Err(DriverError::Connection(_) | DriverError::Timeout)
    ));
}
