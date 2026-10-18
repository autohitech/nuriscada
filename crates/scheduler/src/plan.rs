// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! 읽기 계획: 같은 장치·유닛·영역의 태그 주소를 묶어 블록으로 읽는다.

use nuriscada_driver_api::{DataKind, ReadBlock};
use nuriscada_project::Project;

/// 블록을 만드는 한도.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanLimits {
    /// 워드 블록 하나의 최대 개수.
    pub max_words: u16,
    /// 비트 블록 하나의 최대 개수.
    pub max_bits: u16,
    /// 두 태그 사이에 허용하는 빈 주소 수(이만큼은 따로 읽는 것보다 같이 읽는 편이 싸다고 본다).
    pub max_gap: u16,
}

impl PlanLimits {
    /// MODBUS TCP의 한 요청 한도(워드 125, 비트 2000)와 빈 주소 8개.
    pub const MODBUS: Self = Self {
        max_words: 125,
        max_bits: 2000,
        max_gap: 8,
    };
}

/// 계획된 읽기 블록과, 그 안에서 값을 가져갈 태그.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedBlock {
    /// 장치 목록에서의 위치.
    pub device: usize,
    /// 읽을 영역.
    pub block: ReadBlock,
    /// (태그 목록에서의 위치, 블록 안 오프셋).
    pub tags: Vec<(usize, u16)>,
}

/// 프로젝트의 태그를 읽기 블록으로 묶는다. 결과는 장치, 유닛, 영역, 주소 순이다.
pub fn plan(project: &Project, limits: PlanLimits) -> Vec<PlannedBlock> {
    let mut order: Vec<usize> = (0..project.tags.len()).collect();
    order.sort_by_key(|&i| {
        let t = &project.tags[i];
        (t.device, t.unit, t.space.code(), t.address)
    });

    let mut blocks: Vec<PlannedBlock> = Vec::new();
    for i in order {
        let t = &project.tags[i];
        let kind = t.space.kind();
        let max = match kind {
            DataKind::Bit => limits.max_bits,
            DataKind::Word => limits.max_words,
        };
        if let Some(last) = blocks.last_mut() {
            let same = last.device == t.device
                && last.block.unit == t.unit
                && last.block.space == t.space.code();
            if same {
                let end = u32::from(last.block.start) + u32::from(last.block.count);
                let addr = u32::from(t.address);
                if addr <= end + u32::from(limits.max_gap) {
                    let new_end = (addr + 1).max(end);
                    let new_count = new_end - u32::from(last.block.start);
                    if new_count <= u32::from(max) {
                        last.block.count = u16::try_from(new_count).expect("한도 이하");
                        last.tags.push((i, t.address - last.block.start));
                        continue;
                    }
                }
            }
        }
        blocks.push(PlannedBlock {
            device: t.device,
            block: ReadBlock {
                unit: t.unit,
                space: t.space.code(),
                start: t.address,
                count: 1,
                kind,
            },
            tags: vec![(i, 0)],
        });
    }
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(tags: &str) -> Project {
        Project::from_toml(&format!(
            r#"
[[device]]
name = "a"
driver = "modbus-tcp"
address = "127.0.0.1:1"
[[device]]
name = "b"
driver = "modbus-tcp"
address = "127.0.0.1:2"
{tags}"#
        ))
        .unwrap()
    }

    fn analog(name: &str, device: &str, space: &str, address: u16) -> String {
        format!(
            "[[tag]]\nname=\"{name}\"\ndevice=\"{device}\"\nspace=\"{space}\"\naddress={address}\ntype=\"analog\"\nscale={{plc_base=0,plc_full=100,base=0,full=100}}\n"
        )
    }

    #[test]
    fn merges_close_addresses() {
        let p = project(
            &[
                analog("t3", "a", "holding", 12),
                analog("t1", "a", "holding", 10),
                analog("t2", "a", "holding", 11),
            ]
            .concat(),
        );
        let blocks = plan(&p, PlanLimits::MODBUS);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].block.start, 10);
        assert_eq!(blocks[0].block.count, 3);
        let names: Vec<(&str, u16)> = blocks[0]
            .tags
            .iter()
            .map(|&(i, off)| (p.tags[i].name.as_str(), off))
            .collect();
        assert_eq!(names, [("t1", 0), ("t2", 1), ("t3", 2)]);
    }

    #[test]
    fn splits_far_addresses_other_areas_and_devices() {
        let p = project(
            &[
                analog("a1", "a", "holding", 0),
                analog("a2", "a", "holding", 100),
                analog("a3", "a", "input", 1),
                analog("b1", "b", "holding", 0),
            ]
            .concat(),
        );
        let blocks = plan(&p, PlanLimits::MODBUS);
        assert_eq!(blocks.len(), 4);
        assert_eq!(blocks.iter().filter(|b| b.device == 1).count(), 1);
    }

    #[test]
    fn respects_block_size_limit() {
        let limits = PlanLimits {
            max_words: 4,
            max_bits: 8,
            max_gap: 0,
        };
        let tags: String = (0..6)
            .map(|i| analog(&format!("t{i}"), "a", "holding", i))
            .collect();
        let blocks = plan(&project(&tags), limits);
        let counts: Vec<u16> = blocks.iter().map(|b| b.block.count).collect();
        assert_eq!(counts, [4, 2]);
    }

    #[test]
    fn same_address_tags_share_one_block() {
        let p = project(
            &[
                analog("x", "a", "holding", 5),
                analog("y", "a", "holding", 5),
            ]
            .concat(),
        );
        let blocks = plan(&p, PlanLimits::MODBUS);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].block.count, 1);
        assert_eq!(blocks[0].tags.len(), 2);
    }

    #[test]
    fn gap_boundary() {
        let limits = PlanLimits {
            max_words: 125,
            max_bits: 2000,
            max_gap: 2,
        };
        // 주소 0, 3: 사이에 빈 주소 2개 → 한 블록. 0, 4: 빈 주소 3개 → 분리.
        let together = project(
            &[
                analog("a", "a", "holding", 0),
                analog("b", "a", "holding", 3),
            ]
            .concat(),
        );
        assert_eq!(plan(&together, limits).len(), 1);
        let apart = project(
            &[
                analog("a", "a", "holding", 0),
                analog("b", "a", "holding", 4),
            ]
            .concat(),
        );
        assert_eq!(plan(&apart, limits).len(), 2);
    }
}
