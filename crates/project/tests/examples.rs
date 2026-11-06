// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! 예제 프로젝트 회귀 시험. 저장소의 `examples/*.toml`이 항상 검증·왕복을 통과해야 한다.

use nuriscada_project::Project;

const EXAMPLES: &[&str] = &[
    "demo.toml",
    "demo-fins.toml",
    "demo-melsec.toml",
    "demo-s7.toml",
    "demo-xgt.toml",
];

#[test]
fn 모든_예제는_검증을_통과하고_왕복한다() {
    for name in EXAMPLES {
        let path = format!("../../examples/{name}");
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let project = Project::from_toml(&text).unwrap_or_else(|e| panic!("{path}: {e:?}"));
        assert!(!project.tags.is_empty(), "{path}: 태그가 없다");
        let reparsed =
            Project::from_toml(&project.to_toml()).unwrap_or_else(|e| panic!("{path}: {e:?}"));
        assert_eq!(project, reparsed, "{path}");
    }
}
