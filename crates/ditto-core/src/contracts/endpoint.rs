#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransportKind {
    Http,
    WebSocket,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Delete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EndpointQueryParam {
    pub name: &'static str,
    pub value_template: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtocolQuirks {
    pub require_model_prefix: bool,
    pub supports_system_role: bool,
    pub force_stream_options: bool,
}

impl ProtocolQuirks {
    pub const NONE: Self = Self {
        require_model_prefix: false,
        supports_system_role: true,
        force_stream_options: false,
    };
}

impl Default for ProtocolQuirks {
    fn default() -> Self {
        Self::NONE
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EndpointTemplate {
    pub transport: TransportKind,
    pub http_method: Option<HttpMethod>,
    pub base_url_override: Option<&'static str>,
    pub path_template: &'static str,
    pub query_params: &'static [EndpointQueryParam],
}

impl EndpointTemplate {
    pub fn render(self, model: &str) -> ResolvedEndpoint {
        ResolvedEndpoint {
            transport: self.transport,
            http_method: self.http_method,
            base_url_override: self
                .base_url_override
                .map(|base_url| render_template(base_url, model)),
            path: render_template(self.path_template, model),
            query_params: self
                .query_params
                .iter()
                .map(|param| {
                    (
                        param.name.to_string(),
                        render_template(param.value_template, model),
                    )
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedEndpoint {
    pub transport: TransportKind,
    pub http_method: Option<HttpMethod>,
    pub base_url_override: Option<String>,
    pub path: String,
    pub query_params: Vec<(String, String)>,
}

fn render_template(template: &str, model: &str) -> String {
    template.replace("{model}", model)
}
