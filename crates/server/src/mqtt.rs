// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! MQTT 읽기 전용 다리(SQ-C06). 태그 묶음을 JSON 그대로 발행한다.
//!
//! 쓰기·제어는 받지 않는다. 브로커가 없어도 서버는 계속 돈다.
//! 평문 MQTT만 지원한다(TLS 의존성을 뺐다. `mqtts://`는 거부한다).
use std::sync::Arc;
use std::time::Duration;

use nuriscada_scheduler::Hub;
use rumqttc::{AsyncClient, MqttOptions, QoS};

/// `mqtt://호스트[:포트]`를 푼다. 포트가 없으면 1883이다. TLS는 지원하지 않는다.
pub fn parse_broker(text: &str) -> Result<(String, u16), String> {
    if text.starts_with("mqtts://") {
        return Err(format!("TLS 브로커(mqtts)는 지원하지 않는다: {text}"));
    }
    let rest = text
        .strip_prefix("mqtt://")
        .ok_or_else(|| format!("브로커 주소는 mqtt://로 시작해야 한다: {text}"))?;
    if rest.is_empty() || rest.contains('/') || rest.contains(' ') {
        return Err(format!("브로커 주소가 올바르지 않다: {text}"));
    }
    match rest.split_once(':') {
        Some((host, port)) => {
            if host.is_empty() {
                return Err(format!("브로커 주소가 올바르지 않다: {text}"));
            }
            let port: u16 = port
                .parse()
                .map_err(|_| format!("브로커 포트가 숫자가 아니다: {text}"))?;
            Ok((host.to_string(), port))
        }
        None => Ok((rest.to_string(), 1883)),
    }
}

/// 발행 주제. 묶음은 `{앞머리}/tags`로 간다.
pub fn tags_topic(prefix: &str) -> String {
    format!("{prefix}/tags")
}

/// 브로커로 태그 묶음을 발행한다. 연결이 끊겨도 되살리기를 rumqttc에 맡기고 계속 돈다.
/// 호출자는 이 태스크를 떼어 놓고 서버는 그대로 진행한다.
pub async fn run(broker: &str, prefix: &str, hub: Arc<Hub>) {
    let (host, port) = match parse_broker(broker) {
        Ok(pair) => pair,
        Err(e) => {
            crate::log::emit("error", "mqtt", &e);
            return;
        }
    };
    let mut options = MqttOptions::new("nuriscada", host, port);
    options.set_keep_alive(Duration::from_secs(15));
    let (client, mut events) = AsyncClient::new(options, 64);
    let topic = tags_topic(prefix);
    let mut wire = hub.subscribe_wire();
    crate::log::emit("info", "mqtt", &format!("발행 시작: {topic}"));
    loop {
        tokio::select! {
            event = events.poll() => {
                if let Err(e) = event {
                    crate::log::emit("warn", "mqtt", &format!("브로커 연결 문제: {e}"));
                }
            }
            received = wire.recv() => {
                match received {
                    Ok(frame) => {
                        if frame.seq.is_none() {
                            continue;
                        }
                        if let Err(e) = client
                            .publish(&topic, QoS::AtLeastOnce, false, frame.text.as_bytes())
                            .await
                        {
                            crate::log::emit("warn", "mqtt", &format!("발행 실패: {e}"));
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 브로커_주소를_푼다() {
        assert_eq!(
            parse_broker("mqtt://example:1884").unwrap(),
            ("example".into(), 1884)
        );
        assert_eq!(
            parse_broker("mqtt://example").unwrap(),
            ("example".into(), 1883)
        );
        assert!(parse_broker("example").is_err());
        assert!(parse_broker("mqtt://").is_err());
        assert!(parse_broker("mqtt://h:notaport").is_err());
        assert!(parse_broker("mqtts://h").is_err());
    }

    #[test]
    fn 주제는_앞머리_slash_tags다() {
        assert_eq!(tags_topic("nuriscada"), "nuriscada/tags");
    }
}
