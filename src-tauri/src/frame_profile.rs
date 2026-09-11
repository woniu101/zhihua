use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FrameAspectRatio {
    Auto,
    #[serde(rename = "16:9")]
    Landscape,
    #[serde(rename = "9:16")]
    Portrait,
    #[serde(rename = "4:3")]
    ClassicLandscape,
    #[serde(rename = "3:4")]
    ClassicPortrait,
    #[serde(rename = "1:1")]
    Square,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameSize {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameProfile {
    pub aspect_ratio: FrameAspectRatio,
    pub work: FrameSize,
    pub visible: FrameSize,
    pub crop_x: u32,
    pub crop_y: u32,
    pub full_hd: FrameSize,
}

impl FrameAspectRatio {
    pub fn from_label(value: &str) -> Option<Self> {
        match value {
            "auto" => Some(Self::Auto),
            "16:9" => Some(Self::Landscape),
            "9:16" => Some(Self::Portrait),
            "4:3" => Some(Self::ClassicLandscape),
            "3:4" => Some(Self::ClassicPortrait),
            "1:1" => Some(Self::Square),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Auto | Self::Landscape => "16:9",
            Self::Portrait => "9:16",
            Self::ClassicLandscape => "4:3",
            Self::ClassicPortrait => "3:4",
            Self::Square => "1:1",
        }
    }

    pub fn profile(self) -> FrameProfile {
        match self {
            Self::Auto | Self::Landscape => FrameProfile {
                aspect_ratio: Self::Landscape,
                work: FrameSize {
                    width: 1344,
                    height: 768,
                },
                visible: FrameSize {
                    width: 1344,
                    height: 756,
                },
                crop_x: 0,
                crop_y: 6,
                full_hd: FrameSize {
                    width: 1920,
                    height: 1080,
                },
            },
            Self::Portrait => FrameProfile {
                aspect_ratio: self,
                work: FrameSize {
                    width: 768,
                    height: 1344,
                },
                visible: FrameSize {
                    width: 756,
                    height: 1344,
                },
                crop_x: 6,
                crop_y: 0,
                full_hd: FrameSize {
                    width: 1080,
                    height: 1920,
                },
            },
            Self::ClassicLandscape => FrameProfile {
                aspect_ratio: self,
                work: FrameSize {
                    width: 1024,
                    height: 768,
                },
                visible: FrameSize {
                    width: 1024,
                    height: 768,
                },
                crop_x: 0,
                crop_y: 0,
                full_hd: FrameSize {
                    width: 1440,
                    height: 1080,
                },
            },
            Self::ClassicPortrait => FrameProfile {
                aspect_ratio: self,
                work: FrameSize {
                    width: 768,
                    height: 1024,
                },
                visible: FrameSize {
                    width: 768,
                    height: 1024,
                },
                crop_x: 0,
                crop_y: 0,
                full_hd: FrameSize {
                    width: 1080,
                    height: 1440,
                },
            },
            Self::Square => FrameProfile {
                aspect_ratio: self,
                work: FrameSize {
                    width: 768,
                    height: 768,
                },
                visible: FrameSize {
                    width: 768,
                    height: 768,
                },
                crop_x: 0,
                crop_y: 0,
                full_hd: FrameSize {
                    width: 1080,
                    height: 1080,
                },
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_exact_visible_frames_and_full_hd_derivatives() {
        let landscape = FrameAspectRatio::Landscape.profile();
        assert_eq!(
            landscape.work,
            FrameSize {
                width: 1344,
                height: 768
            }
        );
        assert_eq!(
            landscape.visible,
            FrameSize {
                width: 1344,
                height: 756
            }
        );
        assert_eq!(landscape.crop_y, 6);
        assert_eq!(
            landscape.full_hd,
            FrameSize {
                width: 1920,
                height: 1080
            }
        );

        let portrait = FrameAspectRatio::Portrait.profile();
        assert_eq!(
            portrait.visible,
            FrameSize {
                width: 756,
                height: 1344
            }
        );
        assert_eq!(
            portrait.full_hd,
            FrameSize {
                width: 1080,
                height: 1920
            }
        );
    }
}
