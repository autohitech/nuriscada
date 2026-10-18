// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! API 오류 코드. 응답은 `{"code":"auth.required","error":"<한국어 기본 문구>","detail":"<선택>"}`다.
//!
//! - `code`는 **안정된 식별자**다. 화면은 이 코드로 현재 언어의 문구를 찾는다. 코드를 바꾸면 API가 깨진다.
//! - `error`는 코드가 없는 클라이언트를 위한 한국어 기본 문구다(하위 호환).
//! - `detail`은 코드로 표현할 수 없는 서버 쪽 설명이다(예: 설정 검증이 알려 주는 구체적 이유). 번역되지 않는다.
//!
//! 새 코드를 추가하면 웹 화면의 언어 사전(`web/src/lib/locales/*.ts`의 `errors`)에도 같은 코드를 넣어야 한다.
//! 웹 시험이 이 파일의 코드 목록과 사전을 대조해 빠진 번역을 잡는다.

use axum::Json;
use axum::http::StatusCode;
use serde_json::{Value, json};

/// 오류 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Code {
    /// 서버 내부 오류.
    Internal,
    /// 공개(읽기 전용) 모드에서는 변경할 수 없다.
    PublicReadOnly,
    /// 다른 출처의 요청.
    CrossOrigin,
    /// 요청 확인 헤더가 없다.
    MissingHeader,
    /// 로그인이 필요하다.
    LoginRequired,
    /// 세션이 만료되었다.
    SessionExpired,
    /// 읽기 전용 시연 서버라 지원하지 않는다.
    ReadOnlyDemo,
    /// 입력이 너무 길다.
    InputTooLong,
    /// 요청이 너무 많다.
    RateLimited,
    /// 계정 또는 비밀번호가 맞지 않는다.
    BadCredentials,
    /// 로그인 시도 제한에 걸렸다.
    LoginLocked,
    /// 관리자 권한이 필요하다.
    AdminRequired,
    /// 계정 변경은 로컬 접속에서만 가능하다.
    AccountsLocalOnly,
    /// 태그 이름이 너무 길다.
    TagNameTooLong,
    /// 태그 이름이 올바르지 않다.
    TagNameInvalid,
    /// 알람 확인 권한이 없거나 로컬 접속이 아니다.
    AckForbidden,
    /// 프로젝트 편집은 로컬 관리자만 가능하다.
    ProjectLocalAdminOnly,
    /// 프로젝트 설정 검증 실패(`detail`에 이유).
    ProjectInvalid,
    /// 가져오기는 로컬 관리자만 가능하다.
    ImportLocalAdminOnly,
    /// 가져오기 모드가 올바르지 않다.
    ImportBadMode,
    /// 가져오기 형식이 올바르지 않다.
    ImportBadFormat,
    /// 가져오기에 대상 장치가 필요하다.
    ImportDeviceRequired,
    /// 현재 역할·접속 경로·서버 설정에서는 쓸 수 없다.
    WriteNotAllowed,
    /// 쓰기 대기열이 가득 찼다.
    WriteQueueFull,
    /// 쓰기가 허용된 태그가 아니다.
    WriteTagNotWritable,
    /// 쓰기 값이 올바르지 않다.
    WriteBadValue,
    /// 태그가 없다.
    TagNotFound,
    /// 통신 품질이 나빠 쓸 수 없다.
    WriteBadQuality,
    /// 쓰기 결과를 확인하지 못했다(`detail`에 이유).
    WriteUnconfirmed,
    /// 허용되지 않은 Host.
    HostNotAllowed,
    /// 허용되지 않은 Origin.
    OriginNotAllowed,
}

impl Code {
    /// 모든 코드(시험이 문서·사전과 대조하는 데 쓴다).
    pub const ALL: &'static [Code] = &[
        Code::Internal,
        Code::PublicReadOnly,
        Code::CrossOrigin,
        Code::MissingHeader,
        Code::LoginRequired,
        Code::SessionExpired,
        Code::ReadOnlyDemo,
        Code::InputTooLong,
        Code::RateLimited,
        Code::BadCredentials,
        Code::LoginLocked,
        Code::AdminRequired,
        Code::AccountsLocalOnly,
        Code::TagNameTooLong,
        Code::TagNameInvalid,
        Code::AckForbidden,
        Code::ProjectLocalAdminOnly,
        Code::ProjectInvalid,
        Code::ImportLocalAdminOnly,
        Code::ImportBadMode,
        Code::ImportBadFormat,
        Code::ImportDeviceRequired,
        Code::WriteNotAllowed,
        Code::WriteQueueFull,
        Code::WriteTagNotWritable,
        Code::WriteBadValue,
        Code::TagNotFound,
        Code::WriteBadQuality,
        Code::WriteUnconfirmed,
        Code::HostNotAllowed,
        Code::OriginNotAllowed,
    ];

    /// 안정된 문자열 식별자.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Internal => "server.internal",
            Self::PublicReadOnly => "request.public_read_only",
            Self::CrossOrigin => "request.cross_origin",
            Self::MissingHeader => "request.missing_header",
            Self::LoginRequired => "auth.required",
            Self::SessionExpired => "auth.session_expired",
            Self::ReadOnlyDemo => "server.read_only_demo",
            Self::InputTooLong => "request.input_too_long",
            Self::RateLimited => "request.rate_limited",
            Self::BadCredentials => "auth.bad_credentials",
            Self::LoginLocked => "auth.locked",
            Self::AdminRequired => "auth.admin_required",
            Self::AccountsLocalOnly => "auth.accounts_local_only",
            Self::TagNameTooLong => "tag.name_too_long",
            Self::TagNameInvalid => "tag.name_invalid",
            Self::AckForbidden => "alarm.ack_forbidden",
            Self::ProjectLocalAdminOnly => "project.local_admin_only",
            Self::ProjectInvalid => "project.invalid",
            Self::ImportLocalAdminOnly => "import.local_admin_only",
            Self::ImportBadMode => "import.bad_mode",
            Self::ImportBadFormat => "import.bad_format",
            Self::ImportDeviceRequired => "import.device_required",
            Self::WriteNotAllowed => "write.not_allowed",
            Self::WriteQueueFull => "write.queue_full",
            Self::WriteTagNotWritable => "write.tag_not_writable",
            Self::WriteBadValue => "write.bad_value",
            Self::TagNotFound => "tag.not_found",
            Self::WriteBadQuality => "write.bad_quality",
            Self::WriteUnconfirmed => "write.unconfirmed",
            Self::HostNotAllowed => "request.host_not_allowed",
            Self::OriginNotAllowed => "request.origin_not_allowed",
        }
    }

    /// 코드가 없는 클라이언트를 위한 한국어 기본 문구.
    pub fn message(self) -> &'static str {
        match self {
            Self::Internal => "서버 저장·처리 오류",
            Self::PublicReadOnly => "공개 모드에서는 변경할 수 없다",
            Self::CrossOrigin => "다른 출처의 요청은 허용하지 않는다",
            Self::MissingHeader => "요청 확인 헤더가 필요하다",
            Self::LoginRequired => "로그인이 필요하다",
            Self::SessionExpired => "세션이 만료되었다",
            Self::ReadOnlyDemo => "읽기 전용 시연 서버다",
            Self::InputTooLong => "입력 길이가 너무 길다",
            Self::RateLimited => "잠시 후 다시 시도하라",
            Self::BadCredentials => "계정 또는 비밀번호를 확인하라",
            Self::LoginLocked => "60초 후 다시 시도하라",
            Self::AdminRequired => "관리자 권한이 필요하다",
            Self::AccountsLocalOnly => "계정 변경은 로컬에서만 가능하다",
            Self::TagNameTooLong => "태그 이름이 너무 길다",
            Self::TagNameInvalid => "올바른 태그 이름이 필요하다",
            Self::AckForbidden => "알람 확인 권한 또는 로컬 접속이 필요하다",
            Self::ProjectLocalAdminOnly => "프로젝트 설정 편집은 로컬 관리자만 가능하다",
            Self::ProjectInvalid => "프로젝트 설정이 올바르지 않다",
            Self::ImportLocalAdminOnly => "프로젝트 가져오기는 로컬 관리자만 가능하다",
            Self::ImportBadMode => "mode는 preview, merge, replace 중 하나여야 한다",
            Self::ImportBadFormat => "format은 simple 또는 tagx여야 한다",
            Self::ImportDeviceRequired => {
                "tagx 가져오기는 태그를 붙일 장치(default_device)가 필요하다"
            }
            Self::WriteNotAllowed => "현재 역할·접속 경로·서버 설정에서는 쓸 수 없다",
            Self::WriteQueueFull => "쓰기 대기열이 가득 찼다",
            Self::WriteTagNotWritable => "쓰기가 허용된 태그가 아니다",
            Self::WriteBadValue => "코일은 true/false, 레지스터는 원시 정수 0~65535를 입력하라",
            Self::TagNotFound => "태그가 없다",
            Self::WriteBadQuality => "통신 품질이 나쁜 태그에는 쓸 수 없다",
            Self::WriteUnconfirmed => "쓰기 결과를 확인하지 못했다(자동 재시도 없음)",
            Self::HostNotAllowed => "허용되지 않은 Host",
            Self::OriginNotAllowed => "허용되지 않은 Origin",
        }
    }

    /// 응답 본문.
    pub fn body(self, detail: Option<&str>) -> Value {
        let mut v = json!({"code": self.as_str(), "error": self.message()});
        if let Some(d) = detail {
            v["detail"] = Value::String(d.to_string());
        }
        v
    }

    /// 상태 코드와 본문을 묶는다.
    pub fn response(self, status: StatusCode, detail: Option<&str>) -> (StatusCode, Json<Value>) {
        (status, Json(self.body(detail)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn codes_are_unique_dotted_lowercase_and_complete() {
        let mut seen = HashSet::new();
        for c in Code::ALL {
            let s = c.as_str();
            assert!(seen.insert(s), "중복 코드: {s}");
            assert!(s.contains('.'), "{s}");
            assert!(
                s.chars()
                    .all(|ch| ch.is_ascii_lowercase() || ch == '.' || ch == '_'),
                "{s}"
            );
            assert!(!c.message().is_empty());
        }
        assert_eq!(seen.len(), Code::ALL.len());
    }

    #[test]
    fn body_has_code_message_and_optional_detail() {
        let v = Code::LoginRequired.body(None);
        assert_eq!(v["code"], "auth.required");
        assert_eq!(v["error"], "로그인이 필요하다");
        assert!(v.get("detail").is_none());
        let v = Code::ProjectInvalid.body(Some("태그 이름이 중복된다"));
        assert_eq!(v["detail"], "태그 이름이 중복된다");
    }
}
