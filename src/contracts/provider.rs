use super::ids::WireProtocol;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VerificationStatus {
    Explicit,
    FamilyInferred,
    DocsOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EvidenceLevel {
    OfficialDocs,
    OfficialSdk,
    OfficialDemo,
    CommunityRepo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderClass {
    GenericOpenAi,
    NativeAnthropic,
    NativeGoogle,
    OpenAiCompatible,
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ProviderProtocolFamily {
    OpenAi,
    Anthropic,
    Google,
    Dashscope,
    Qianfan,
    Ark,
    Hunyuan,
    Minimax,
    Zhipu,
    Custom,
    Mixed,
    #[default]
    Unknown,
}

impl ProviderProtocolFamily {
    pub fn from_provider_class(class: ProviderClass) -> Self {
        match class {
            ProviderClass::GenericOpenAi | ProviderClass::OpenAiCompatible => Self::OpenAi,
            ProviderClass::NativeAnthropic => Self::Anthropic,
            ProviderClass::NativeGoogle => Self::Google,
            ProviderClass::Custom => Self::Custom,
        }
    }

    pub fn from_wire_protocol(wire_protocol: WireProtocol) -> Self {
        let wire = wire_protocol.as_str();
        if wire.starts_with("openai.") {
            Self::OpenAi
        } else if wire.starts_with("anthropic.") {
            Self::Anthropic
        } else if wire.starts_with("google.") {
            Self::Google
        } else if wire.starts_with("dashscope.") {
            Self::Dashscope
        } else if wire.starts_with("qianfan.") {
            Self::Qianfan
        } else if wire.starts_with("ark.") {
            Self::Ark
        } else if wire.starts_with("hunyuan.") {
            Self::Hunyuan
        } else if wire.starts_with("minimax.") {
            Self::Minimax
        } else if wire.starts_with("zhipu.") {
            Self::Zhipu
        } else {
            Self::Unknown
        }
    }

    pub fn merge(self, other: Self) -> Self {
        match (self, other) {
            (Self::Unknown, family) | (family, Self::Unknown) => family,
            (left, right) if left == right => left,
            _ => Self::Mixed,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvidenceRef {
    pub level: EvidenceLevel,
    pub source_url: &'static str,
    pub note: Option<&'static str>,
}
