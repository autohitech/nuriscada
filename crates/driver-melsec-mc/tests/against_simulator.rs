// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! 시뮬레이터를 상대로 한 드라이버 통합 시험.

use std::time::Duration;

use nuriscada_driver_api::{BlockData, DataKind, Driver, DriverError, LinkState, ReadBlock};
use nuriscada_driver_melsec_mc::{
    MelsecMcClient, SPACE_D, SPACE_M, SPACE_R, SPACE_W, SPACE_X, SPACE_Y,
};
use nuriscada_sim_melsec::Simulator;

fn block(space: u16, start: u16, count: u16, kind: DataKind) -> ReadBlock {
    ReadBlock {
        unit: 0,
        space,
        start,
        count,
        kind,
    }
}

async fn setup() -> (Simulator, MelsecMcClient) {
    let sim = Simulator::start("127.0.0.1:0", 200, 128).await.unwrap();
    let client = MelsecMcClient::new(sim.addr().to_string(), Duration::from_secs(2));
    (sim, client)
}

#[tokio::test]
async fn 워드_세_영역을_읽는다() {
    let (sim, mut client) = setup().await;
    sim.set_d(0, &[0x1234, 0x5678]);
    sim.state().w[10] = 42;
    sim.state().r[20] = 99;
    assert_eq!(
        client
            .read(&block(SPACE_D, 0, 2, DataKind::Word))
            .await
            .unwrap(),
        BlockData::Words(vec![0x1234, 0x5678])
    );
    assert_eq!(
        client
            .read(&block(SPACE_W, 10, 1, DataKind::Word))
            .await
            .unwrap(),
        BlockData::Words(vec![42])
    );
    assert_eq!(
        client
            .read(&block(SPACE_R, 20, 1, DataKind::Word))
            .await
            .unwrap(),
        BlockData::Words(vec![99])
    );
    assert_eq!(client.status().state, LinkState::Connected);
}

#[tokio::test]
async fn 비트_세_영역을_읽는다() {
    let (sim, mut client) = setup().await;
    sim.set_m(0, &[true, false, true]);
    sim.state().x[0x10] = true;
    sim.state().y[5] = true;
    assert_eq!(
        client
            .read(&block(SPACE_M, 0, 3, DataKind::Bit))
            .await
            .unwrap(),
        BlockData::Bits(vec![true, false, true])
    );
    // X는 16진 주소: X10 = 0x10.
    assert_eq!(
        client
            .read(&block(SPACE_X, 0x10, 1, DataKind::Bit))
            .await
            .unwrap(),
        BlockData::Bits(vec![true])
    );
    assert_eq!(
        client
            .read(&block(SPACE_Y, 5, 1, DataKind::Bit))
            .await
            .unwrap(),
        BlockData::Bits(vec![true])
    );
}

#[tokio::test]
async fn 바뀌는_값을_읽는다() {
    let (sim, mut client) = setup().await;
    sim.set_d(0, &[1]);
    assert_eq!(
        client
            .read(&block(SPACE_D, 0, 1, DataKind::Word))
            .await
            .unwrap(),
        BlockData::Words(vec![1])
    );
    sim.set_d(0, &[2]);
    assert_eq!(
        client
            .read(&block(SPACE_D, 0, 1, DataKind::Word))
            .await
            .unwrap(),
        BlockData::Words(vec![2])
    );
}

#[tokio::test]
async fn 장치_오류는_연결을_유지한다() {
    let (_sim, mut client) = setup().await;
    // 범위를 벗어난 D 주소.
    let err = client
        .read(&block(SPACE_D, 190, 20, DataKind::Word))
        .await
        .unwrap_err();
    assert!(matches!(err, DriverError::Device { .. }));
    assert_eq!(client.status().state, LinkState::Connected);
}

#[tokio::test]
async fn 잘못된_요청은_미리_거부한다() {
    let (_sim, mut client) = setup().await;
    // 모르는 영역.
    let err = client
        .read(&block(999, 0, 1, DataKind::Word))
        .await
        .unwrap_err();
    assert!(matches!(err, DriverError::InvalidRequest(_)));
    // D 영역에 비트 종류.
    let err = client
        .read(&block(SPACE_D, 0, 1, DataKind::Bit))
        .await
        .unwrap_err();
    assert!(matches!(err, DriverError::InvalidRequest(_)));
    // 개수 0.
    let err = client
        .read(&block(SPACE_D, 0, 0, DataKind::Word))
        .await
        .unwrap_err();
    assert!(matches!(err, DriverError::InvalidRequest(_)));
}

#[tokio::test]
async fn 느린_장치는_시간_초과_후_회복한다() {
    let (sim, mut client) = setup().await;
    sim.set_d(0, &[7]);
    sim.state().response_delay = Duration::from_secs(5);
    let err = client
        .read(&block(SPACE_D, 0, 1, DataKind::Word))
        .await
        .unwrap_err();
    assert!(matches!(err, DriverError::Timeout));
    sim.state().response_delay = Duration::ZERO;
    assert_eq!(
        client
            .read(&block(SPACE_D, 0, 1, DataKind::Word))
            .await
            .unwrap(),
        BlockData::Words(vec![7])
    );
}

#[tokio::test]
async fn 끊긴_연결은_보고하고_회복한다() {
    let (sim, mut client) = setup().await;
    sim.set_d(0, &[7]);
    sim.state().drop_connections = true;
    assert!(matches!(
        client.read(&block(SPACE_D, 0, 1, DataKind::Word)).await,
        Err(DriverError::Connection(_))
    ));
    sim.state().drop_connections = false;
    assert_eq!(
        client
            .read(&block(SPACE_D, 0, 1, DataKind::Word))
            .await
            .unwrap(),
        BlockData::Words(vec![7])
    );
}

#[tokio::test]
async fn 연결_실패는_연결_오류다() {
    let mut client = MelsecMcClient::new("127.0.0.1:1", Duration::from_millis(300));
    assert!(matches!(
        client.connect().await,
        Err(DriverError::Connection(_) | DriverError::Timeout)
    ));
}

#[tokio::test]
async fn 단일_d와_m_쓰기_후_읽힌다() {
    let (sim, mut client) = setup().await;
    client.write_single(0, SPACE_D, 5, 321).await.unwrap();
    assert_eq!(
        client
            .read(&block(SPACE_D, 5, 1, DataKind::Word))
            .await
            .unwrap(),
        BlockData::Words(vec![321])
    );
    client.write_single(0, SPACE_M, 3, 1).await.unwrap();
    assert_eq!(
        client
            .read(&block(SPACE_M, 3, 1, DataKind::Bit))
            .await
            .unwrap(),
        BlockData::Bits(vec![true])
    );
    client.write_single(0, SPACE_M, 3, 0).await.unwrap();
    assert_eq!(
        client
            .read(&block(SPACE_M, 3, 1, DataKind::Bit))
            .await
            .unwrap(),
        BlockData::Bits(vec![false])
    );
    assert!(sim.state().d[5] == 321 && !sim.state().m[3]);
}

#[tokio::test]
async fn 지원하지_않는_영역_쓰기는_거부한다() {
    let (_sim, mut client) = setup().await;
    // 존재하지 않는 영역 번호는 거부한다.
    assert!(matches!(
        client.write_single(0, 999, 0, 1).await,
        Err(DriverError::InvalidRequest(_))
    ));
    assert!(matches!(
        client.write_single(0, 3, 0, 1).await,
        Err(DriverError::InvalidRequest(_))
    ));
}

#[tokio::test]
async fn 범위를_벗어난_쓰기는_장치_오류다() {
    let (_sim, mut client) = setup().await;
    assert!(matches!(
        client.write_single(0, SPACE_D, 200, 1).await,
        Err(DriverError::Device { .. })
    ));
}

#[tokio::test]
async fn wrxy_쓰기_후_읽힌다() {
    use nuriscada_driver_melsec_mc::{SPACE_R, SPACE_W, SPACE_X, SPACE_Y};
    let (sim, mut client) = setup().await;
    client.write_single(0, SPACE_W, 0, 11).await.unwrap();
    client.write_single(0, SPACE_R, 1, 22).await.unwrap();
    client.write_single(0, SPACE_X, 0, 1).await.unwrap();
    client.write_single(0, SPACE_Y, 1, 1).await.unwrap();
    assert_eq!(
        client
            .read(&block(SPACE_W, 0, 2, DataKind::Word))
            .await
            .unwrap(),
        BlockData::Words(vec![11, 0])
    );
    assert_eq!(
        client
            .read(&block(SPACE_R, 1, 1, DataKind::Word))
            .await
            .unwrap(),
        BlockData::Words(vec![22])
    );
    assert_eq!(
        client
            .read(&block(SPACE_X, 0, 1, DataKind::Bit))
            .await
            .unwrap(),
        BlockData::Bits(vec![true])
    );
    assert_eq!(
        client
            .read(&block(SPACE_Y, 1, 1, DataKind::Bit))
            .await
            .unwrap(),
        BlockData::Bits(vec![true])
    );
    assert!(sim.state().w[0] == 11 && sim.state().r[1] == 22);
}

#[tokio::test]
async fn 다중_d와_m_쓰기_후_읽힌다() {
    let (sim, mut client) = setup().await;
    client
        .write_multiple(0, SPACE_D, 0, &[11, 22], &[])
        .await
        .unwrap();
    client
        .write_multiple(0, SPACE_M, 0, &[], &[true, false, true])
        .await
        .unwrap();
    assert_eq!(
        client
            .read(&block(SPACE_D, 0, 2, DataKind::Word))
            .await
            .unwrap(),
        BlockData::Words(vec![11, 22])
    );
    assert_eq!(
        client
            .read(&block(SPACE_M, 0, 3, DataKind::Bit))
            .await
            .unwrap(),
        BlockData::Bits(vec![true, false, true])
    );
    assert_eq!(sim.state().d[0], 11);
    assert_eq!(sim.state().d[1], 22);
    assert_eq!(&sim.state().m[0..3], &[true, false, true]);
}
