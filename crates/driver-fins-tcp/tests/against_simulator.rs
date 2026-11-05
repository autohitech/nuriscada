// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! 시뮬레이터를 상대로 한 드라이버 통합 시험.

use std::time::Duration;

use nuriscada_driver_api::{BlockData, DataKind, Driver, DriverError, LinkState, ReadBlock};
use nuriscada_driver_fins_tcp::{FinsTcpClient, SPACE_CIO, SPACE_DM};
use nuriscada_sim_fins::Simulator;

fn block(space: u16, start: u16, count: u16, kind: DataKind) -> ReadBlock {
    ReadBlock {
        unit: 0,
        space,
        start,
        count,
        kind,
    }
}

async fn setup() -> (Simulator, FinsTcpClient) {
    let sim = Simulator::start("127.0.0.1:0", 300, 128).await.unwrap();
    let client = FinsTcpClient::new(sim.addr().to_string(), Duration::from_secs(2));
    (sim, client)
}

#[tokio::test]
async fn dm_워드를_읽는다() {
    let (sim, mut client) = setup().await;
    sim.set_dm(20, &[0x1234, 0x5678]);
    assert_eq!(
        client
            .read(&block(SPACE_DM, 20, 2, DataKind::Word))
            .await
            .unwrap(),
        BlockData::Words(vec![0x1234, 0x5678])
    );
    assert_eq!(client.status().state, LinkState::Connected);
}

#[tokio::test]
async fn cio_비트를_읽는다() {
    let (sim, mut client) = setup().await;
    // CIO 1.03: 비트 번호 1*16+3 = 19.
    sim.set_cio(1, &[0x0008]);
    assert_eq!(
        client
            .read(&block(SPACE_CIO, 19, 2, DataKind::Bit))
            .await
            .unwrap(),
        BlockData::Bits(vec![true, false])
    );
}

#[tokio::test]
async fn 바뀌는_값을_읽는다() {
    let (sim, mut client) = setup().await;
    sim.set_dm(0, &[1]);
    assert_eq!(
        client
            .read(&block(SPACE_DM, 0, 1, DataKind::Word))
            .await
            .unwrap(),
        BlockData::Words(vec![1])
    );
    sim.set_dm(0, &[2]);
    assert_eq!(
        client
            .read(&block(SPACE_DM, 0, 1, DataKind::Word))
            .await
            .unwrap(),
        BlockData::Words(vec![2])
    );
}

#[tokio::test]
async fn 장치_오류는_연결을_유지한다() {
    let (_sim, mut client) = setup().await;
    // 범위를 벗어난 DM 주소.
    let err = client
        .read(&block(SPACE_DM, 295, 10, DataKind::Word))
        .await
        .unwrap_err();
    assert!(matches!(err, DriverError::Device { .. }));
    assert_eq!(client.status().state, LinkState::Connected);
}

#[tokio::test]
async fn 잘못된_요청은_미리_거부한다() {
    let (_sim, mut client) = setup().await;
    assert!(matches!(
        client.read(&block(999, 0, 1, DataKind::Word)).await,
        Err(DriverError::InvalidRequest(_))
    ));
    assert!(matches!(
        client.read(&block(SPACE_DM, 0, 1, DataKind::Bit)).await,
        Err(DriverError::InvalidRequest(_))
    ));
    assert!(matches!(
        client.read(&block(SPACE_DM, 0, 0, DataKind::Word)).await,
        Err(DriverError::InvalidRequest(_))
    ));
}

#[tokio::test]
async fn 느린_장치는_시간_초과_후_회복한다() {
    let (sim, mut client) = setup().await;
    sim.set_dm(0, &[7]);
    sim.state().response_delay = Duration::from_secs(5);
    assert!(matches!(
        client.read(&block(SPACE_DM, 0, 1, DataKind::Word)).await,
        Err(DriverError::Timeout)
    ));
    sim.state().response_delay = Duration::ZERO;
    assert_eq!(
        client
            .read(&block(SPACE_DM, 0, 1, DataKind::Word))
            .await
            .unwrap(),
        BlockData::Words(vec![7])
    );
}

#[tokio::test]
async fn 끊긴_연결은_보고하고_회복한다() {
    let (sim, mut client) = setup().await;
    sim.set_dm(0, &[7]);
    sim.state().drop_connections = true;
    assert!(matches!(
        client.read(&block(SPACE_DM, 0, 1, DataKind::Word)).await,
        Err(DriverError::Connection(_))
    ));
    sim.state().drop_connections = false;
    assert_eq!(
        client
            .read(&block(SPACE_DM, 0, 1, DataKind::Word))
            .await
            .unwrap(),
        BlockData::Words(vec![7])
    );
}

#[tokio::test]
async fn 연결_실패는_연결_오류다() {
    let mut client = FinsTcpClient::new("127.0.0.1:1", Duration::from_millis(300));
    assert!(matches!(
        client.connect().await,
        Err(DriverError::Connection(_) | DriverError::Timeout)
    ));
}
