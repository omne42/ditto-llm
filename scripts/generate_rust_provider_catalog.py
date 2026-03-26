#!/usr/bin/env python3
from __future__ import annotations

import json
import urllib.parse
from collections import defaultdict
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
DITTO_CORE_DIR = ROOT / 'crates' / 'ditto-core'
SOURCE_DIR = ROOT / 'catalog' / 'provider_models'
TARGET_DIR = DITTO_CORE_DIR / 'src' / 'catalog' / 'generated'
TARGET_MODULE_DIR = TARGET_DIR / 'providers'
LEGACY_TARGET_FILE = TARGET_DIR / 'providers.rs'
CONTRACT_IDS_TARGET_FILE = DITTO_CORE_DIR / 'src' / 'contracts' / 'ids.rs'

SKIP_PROVIDERS: set[str] = set()

PROVIDER_FEATURES = {
    'openai': 'provider-openai',
    'anthropic': 'provider-anthropic',
    'google': 'provider-google',
    'bailian': 'provider-bailian',
    'deepseek': 'provider-deepseek',
    'doubao': 'provider-doubao',
    'hunyuan': 'provider-hunyuan',
    'kimi': 'provider-kimi',
    'minimax': 'provider-minimax',
    'openrouter': 'provider-openrouter',
    'qianfan': 'provider-qianfan',
    'xai': 'provider-xai',
    'zhipu': 'provider-zhipu',
}

PROVIDER_CLASS = {
    'openai': 'ProviderClass::GenericOpenAi',
    'anthropic': 'ProviderClass::NativeAnthropic',
    'google': 'ProviderClass::NativeGoogle',
    'deepseek': 'ProviderClass::OpenAiCompatible',
    'kimi': 'ProviderClass::OpenAiCompatible',
    'openrouter': 'ProviderClass::OpenAiCompatible',
    'xai': 'ProviderClass::OpenAiCompatible',
}

OPERATION_SPECS = [
    ('CHAT_COMPLETION', 'chat.completion', 'LLM', 'LLM'),
    ('RESPONSE', 'response', 'LLM', 'LLM'),
    ('TEXT_COMPLETION', 'text.completion', 'LLM', 'LLM'),
    ('EMBEDDING', 'embedding', 'EMBEDDING', 'EMBEDDING'),
    ('MULTIMODAL_EMBEDDING', 'embedding.multimodal', 'EMBEDDING', 'EMBEDDING'),
    ('IMAGE_GENERATION', 'image.generation', 'IMAGE_GENERATION', 'IMAGE_GENERATION'),
    ('IMAGE_EDIT', 'image.edit', 'IMAGE_EDIT', 'IMAGE_EDIT'),
    ('IMAGE_TRANSLATION', 'image.translation', 'IMAGE_TRANSLATION', 'IMAGE_TRANSLATION'),
    ('IMAGE_QUESTION', 'image.question', 'IMAGE_QUESTION', 'IMAGE_QUESTION'),
    ('VIDEO_GENERATION', 'video.generation', 'VIDEO_GENERATION', 'VIDEO_GENERATION'),
    ('AUDIO_SPEECH', 'audio.speech', 'AUDIO_SPEECH', 'AUDIO_SPEECH'),
    ('AUDIO_TRANSCRIPTION', 'audio.transcription', 'AUDIO_TRANSCRIPTION', 'AUDIO_TRANSCRIPTION'),
    ('AUDIO_TRANSLATION', 'audio.translation', 'AUDIO_TRANSLATION', 'AUDIO_TRANSLATION'),
    ('AUDIO_VOICE_CLONE', 'audio.voice_clone', 'AUDIO_VOICE_CLONE', 'AUDIO_VOICE_CLONE'),
    ('AUDIO_VOICE_DESIGN', 'audio.voice_design', 'AUDIO_VOICE_DESIGN', 'AUDIO_VOICE_DESIGN'),
    ('REALTIME_SESSION', 'realtime.session', 'REALTIME', 'REALTIME'),
    ('RERANK', 'rerank', 'RERANK', 'RERANK'),
    (
        'CLASSIFICATION_OR_EXTRACTION',
        'classification_or_extraction',
        'LLM',
        'CLASSIFICATION_OR_EXTRACTION',
    ),
    ('MODERATION', 'moderation', 'MODERATION', 'MODERATION'),
    ('BATCH', 'batch', 'BATCH', 'BATCH'),
    ('OCR', 'ocr', 'OCR', 'OCR'),
    ('MODEL_LIST', 'model.list', 'MODEL_LIST', 'MODEL_LIST'),
    ('CONTEXT_CACHE', 'context.cache', 'CONTEXT_CACHE', 'CONTEXT_CACHE'),
    ('THREAD_RUN', 'thread.run', 'LLM', 'LLM'),
    ('GROUP_CHAT_COMPLETION', 'group.chat.completion', 'LLM', 'LLM'),
    ('CHAT_TRANSLATION', 'chat.translation', 'LLM', 'LLM'),
    ('MUSIC_GENERATION', 'music.generation', 'MUSIC_GENERATION', 'MUSIC_GENERATION'),
    ('THREE_D_GENERATION', '3d.generation', 'THREE_D_GENERATION', 'THREE_D_GENERATION'),
]

CAPABILITY_SPECS = [
    ('LLM', 'llm', ['llm'], ['chat.completion', 'response', 'text.completion']),
    ('EMBEDDING', 'embedding', ['embedding', 'embeddings'], ['embedding']),
    (
        'IMAGE_GENERATION',
        'image.generation',
        ['image.generation', 'image_generation', 'image-generation'],
        ['image.generation'],
    ),
    ('IMAGE_EDIT', 'image.edit', ['image.edit', 'image_edit', 'image-edit'], ['image.edit']),
    (
        'IMAGE_TRANSLATION',
        'image.translation',
        ['image.translation', 'image_translation', 'image-translation'],
        [],
    ),
    (
        'IMAGE_QUESTION',
        'image.question',
        ['image.question', 'image_question', 'image-question'],
        [],
    ),
    (
        'VIDEO_GENERATION',
        'video.generation',
        ['video.generation', 'video_generation', 'video-generation'],
        ['video.generation'],
    ),
    ('AUDIO_SPEECH', 'audio.speech', ['audio.speech', 'audio_speech', 'audio-speech'], ['audio.speech']),
    (
        'AUDIO_TRANSCRIPTION',
        'audio.transcription',
        ['audio.transcription', 'audio_transcription', 'audio-transcription'],
        ['audio.transcription'],
    ),
    (
        'AUDIO_TRANSLATION',
        'audio.translation',
        ['audio.translation', 'audio_translation', 'audio-translation'],
        [],
    ),
    (
        'AUDIO_VOICE_CLONE',
        'audio.voice_clone',
        ['audio.voice_clone', 'audio_voice_clone', 'audio-voice-clone'],
        [],
    ),
    (
        'AUDIO_VOICE_DESIGN',
        'audio.voice_design',
        ['audio.voice_design', 'audio_voice_design', 'audio-voice-design'],
        [],
    ),
    ('REALTIME', 'realtime', ['realtime'], ['realtime.session']),
    ('RERANK', 'rerank', ['rerank'], ['rerank']),
    (
        'CLASSIFICATION_OR_EXTRACTION',
        'classification_or_extraction',
        ['classification_or_extraction', 'classification-or-extraction'],
        [],
    ),
    ('MODERATION', 'moderation', ['moderation', 'moderations'], ['moderation']),
    ('BATCH', 'batch', ['batch', 'batches'], ['batch']),
    ('OCR', 'ocr', ['ocr'], []),
    ('MODEL_LIST', 'model.list', ['model.list', 'model_list', 'model-list', 'models'], []),
    (
        'CONTEXT_CACHE',
        'context.cache',
        ['context.cache', 'context_cache', 'context-cache'],
        [],
    ),
    (
        'MUSIC_GENERATION',
        'music.generation',
        ['music.generation', 'music_generation', 'music-generation'],
        [],
    ),
    (
        'THREE_D_GENERATION',
        '3d.generation',
        ['3d.generation', '3d_generation', '3d-generation'],
        [],
    ),
]

SURFACE_SPECS = [
    ('OPENAI_CHAT_COMPLETIONS', 'chat.completion'),
    ('OPENAI_RESPONSES', 'responses'),
    ('OPENAI_TEXT_COMPLETIONS', 'completion.legacy'),
    ('OPENAI_EMBEDDINGS', 'embedding'),
    ('OPENAI_IMAGES_GENERATIONS', 'image.generation'),
    ('OPENAI_IMAGES_EDITS', 'image.edit'),
    ('OPENAI_VIDEOS', 'video.generation.async'),
    ('OPENAI_AUDIO_SPEECH', 'audio.speech'),
    ('OPENAI_AUDIO_TRANSCRIPTIONS', 'audio.transcription'),
    ('OPENAI_AUDIO_TRANSLATIONS', 'audio.translation'),
    ('OPENAI_MODERATIONS', 'moderation'),
    ('OPENAI_BATCHES', 'batch'),
    ('OPENAI_REALTIME', 'realtime.websocket'),
    ('ANTHROPIC_MESSAGES', 'anthropic.messages'),
    ('GOOGLE_GENERATE_CONTENT', 'generate.content'),
    ('GOOGLE_STREAM_GENERATE_CONTENT', 'generate.content.stream'),
    ('GOOGLE_BATCH_GENERATE_CONTENT', 'generate.content.batch'),
    ('GOOGLE_EMBED_CONTENT', 'embedding'),
    ('GOOGLE_BATCH_EMBED_CONTENT', 'embedding.batch'),
    ('GOOGLE_LIVE', 'realtime.websocket'),
    ('GOOGLE_PREDICT', 'image.generation'),
    ('GOOGLE_PREDICT_LONG_RUNNING', 'video.generation'),
]

WIRE_PROTOCOL_SPECS = [
    ('OPENAI_CHAT_COMPLETIONS', 'openai.chat_completions'),
    ('OPENAI_RESPONSES', 'openai.responses'),
    ('OPENAI_TEXT_COMPLETIONS', 'openai.text_completions'),
    ('OPENAI_EMBEDDINGS', 'openai.embeddings'),
    ('OPENAI_IMAGES', 'openai.images'),
    ('OPENAI_VIDEOS', 'openai.videos'),
    ('OPENAI_AUDIO', 'openai.audio'),
    ('OPENAI_MODERATIONS', 'openai.moderations'),
    ('OPENAI_BATCHES', 'openai.batches'),
    ('OPENAI_REALTIME', 'openai.realtime'),
    ('ANTHROPIC_MESSAGES', 'anthropic.messages'),
    ('GOOGLE_GENERATE_CONTENT', 'google.generate_content'),
    ('GOOGLE_EMBED_CONTENT', 'google.embed_content'),
    ('GOOGLE_LIVE', 'google.live'),
    ('GOOGLE_PREDICT', 'google.predict'),
    ('GOOGLE_PREDICT_LONG_RUNNING', 'google.predict_long_running'),
    ('DASHSCOPE_NATIVE', 'dashscope.native'),
    ('DASHSCOPE_INFERENCE_WS', 'dashscope.inference_ws'),
    ('DASHSCOPE_REALTIME_WS', 'dashscope.realtime_ws'),
    ('QIANFAN_NATIVE', 'qianfan.native'),
    ('ARK_NATIVE', 'ark.native'),
    ('HUNYUAN_NATIVE', 'hunyuan.native'),
    ('MINIMAX_NATIVE', 'minimax.native'),
    ('ZHIPU_NATIVE', 'zhipu.native'),
]

CONTEXT_CACHE_MODE_SPECS = [
    ('PASSIVE', 'passive'),
    ('PROMPT_CACHE_KEY', 'prompt_cache_key'),
    ('ANTHROPIC_COMPATIBLE', 'anthropic_compatible'),
]

OPERATION_CONSTS = {
    operation_id: f'OperationKind::{const_name}'
    for const_name, operation_id, _, _ in OPERATION_SPECS
}
CAPABILITY_STATUS_CONSTS = {
    operation_id: f'CapabilityKind::{status_capability_const}'
    for _, operation_id, _, status_capability_const in OPERATION_SPECS
}
SURFACE_CONSTS = {
    surface_id: f'ApiSurfaceId::{const_name}'
    for const_name, surface_id in SURFACE_SPECS
}
WIRE_PROTOCOL_CONSTS = {
    protocol_id: f'WireProtocol::{const_name}'
    for const_name, protocol_id in WIRE_PROTOCOL_SPECS
}
CONTEXT_CACHE_MODE_CONSTS = {
    mode_id: f'ContextCacheModeId::{const_name}'
    for const_name, mode_id in CONTEXT_CACHE_MODE_SPECS
}

OPERATION_BY_SURFACE = {
    'responses': 'response',
    'response.create.beta': 'response',
    'chat.completion': 'chat.completion',
    'group.chat.completion': 'group.chat.completion',
    'anthropic.messages': 'chat.completion',
    'generate.content': 'chat.completion',
    'minimax.chatcompletion_v2': 'chat.completion',
    'completion.legacy': 'text.completion',
    'completion.fim.beta': 'text.completion',
    'embedding': 'embedding',
    'embedding.multimodal': 'embedding.multimodal',
    'image.generation': 'image.generation',
    'image.generation.async': 'image.generation',
    'image.edit': 'image.edit',
    'image.translation': 'image.translation',
    'image.question': 'image.question',
    'video.generation': 'video.generation',
    'video.generation.async': 'video.generation',
    'audio.speech': 'audio.speech',
    'audio.speech.async': 'audio.speech',
    'audio.transcription': 'audio.transcription',
    'audio.transcription.realtime': 'audio.transcription',
    'audio.voice_clone': 'audio.voice_clone',
    'audio.voice_cloning': 'audio.voice_clone',
    'audio.voice_design': 'audio.voice_design',
    'music.generation': 'music.generation',
    'rerank': 'rerank',
    'classification_or_extraction': 'classification_or_extraction',
    'moderation': 'moderation',
    'batch': 'batch',
    'ocr': 'ocr',
    'realtime.websocket': 'realtime.session',
    'thread.run': 'thread.run',
    'chat.translation': 'chat.translation',
    'context.cache': 'context.cache',
    'model.list': 'model.list',
    '3d.generation': '3d.generation',
}

BEHAVIOR_SUPPORT_CONSTS = {
    'unknown': 'BehaviorSupport::Unknown',
    'unsupported': 'BehaviorSupport::Unsupported',
    'supported': 'BehaviorSupport::Supported',
}

ASSISTANT_TOOL_FOLLOWUP_CONSTS = {
    'none': 'AssistantToolFollowupRequirement::None',
    'requires_reasoning_content': 'AssistantToolFollowupRequirement::RequiresReasoningContent',
    'requires_thought_signature': 'AssistantToolFollowupRequirement::RequiresThoughtSignature',
}

REASONING_OUTPUT_CONSTS = {
    'unsupported': 'ReasoningOutputMode::Unsupported',
    'optional': 'ReasoningOutputMode::Optional',
    'always': 'ReasoningOutputMode::Always',
}

REASONING_ACTIVATION_CONSTS = {
    'unavailable': 'ReasoningActivationKind::Unavailable',
    'openai_reasoning_effort': 'ReasoningActivationKind::OpenAiReasoningEffort',
    'deepseek_thinking_type_enabled': 'ReasoningActivationKind::DeepSeekThinkingTypeEnabled',
    'always_on': 'ReasoningActivationKind::AlwaysOn',
}

CACHE_USAGE_REPORTING_CONSTS = {
    'unknown': 'CacheUsageReportingKind::Unknown',
    'standard_usage': 'CacheUsageReportingKind::StandardUsage',
    'deepseek_prompt_cache_hit_miss': 'CacheUsageReportingKind::DeepSeekPromptCacheHitMiss',
}

AUTH_KIND_MAP = {
    'api_key_env': ['AuthMethodKind::ApiKeyHeader'],
    'query_param_env': ['AuthMethodKind::ApiKeyQuery'],
}

DOUBAO_ENDPOINTS = {
    'responses': ('POST', 'https://ark.cn-beijing.volces.com/api/v3/responses'),
    'chat.completion': ('POST', 'https://ark.cn-beijing.volces.com/api/v3/chat/completions'),
    'embedding': ('POST', 'https://ark.cn-beijing.volces.com/api/v3/embeddings'),
    'embedding.multimodal': ('POST', 'https://ark.cn-beijing.volces.com/api/v3/embeddings/multimodal'),
    'context.cache': ('POST', 'https://ark.cn-beijing.volces.com/api/v3/context/create'),
    'batch': ('POST', 'https://ark.cn-beijing.volces.com/api/v3/batch/chat/completions'),
    'image.generation': ('POST', 'https://ark.cn-beijing.volces.com/api/v3/images/generations'),
    'video.generation': ('POST', 'https://ark.cn-beijing.volces.com/api/v3/contents/generations/tasks'),
    '3d.generation': ('POST', 'https://ark.cn-beijing.volces.com/api/v3/contents/generations/tasks'),
}

QIANFAN_ENDPOINTS = {
    'chat.completion': ('POST', 'https://qianfan.baidubce.com/v2/chat/completions'),
    'embedding': ('POST', 'https://qianfan.baidubce.com/v2/embeddings'),
    'embedding.multimodal': ('POST', 'https://qianfan.baidubce.com/v2/embeddings'),
    'rerank': ('POST', 'https://qianfan.baidubce.com/v2/rerank'),
    'image.generation': ('POST', 'https://qianfan.baidubce.com/v2/images/generations'),
    'image.edit': ('POST', 'https://qianfan.baidubce.com/v2/images/edits'),
    'ocr': ('POST', 'https://qianfan.baidubce.com/v2/chat/completions'),
    'video.generation': ('POST', 'https://qianfan.baidubce.com/beta/video/generations/qianfan-video'),
}

HUNYUAN_ENDPOINTS = {
    'chat.completion': ('POST', 'https://api.hunyuan.cloud.tencent.com/v1/chat/completions'),
    'anthropic.messages': ('POST', 'https://api.hunyuan.cloud.tencent.com/anthropic/v1/messages'),
    'embedding': ('POST', 'https://api.hunyuan.cloud.tencent.com/v1/embeddings'),
    'thread.run': ('POST', 'https://hunyuan.tencentcloudapi.com'),
    'image.generation': ('POST', 'https://hunyuan.tencentcloudapi.com'),
    'image.generation.async': ('POST', 'https://hunyuan.tencentcloudapi.com'),
    'group.chat.completion': ('POST', 'https://hunyuan.tencentcloudapi.com'),
    'chat.translation': ('POST', 'https://hunyuan.tencentcloudapi.com'),
    'image.question': ('POST', 'https://hunyuan.tencentcloudapi.com'),
}

ZHIPU_ENDPOINTS = {
    'chat.completion': ('POST', 'https://open.bigmodel.cn/api/paas/v4/chat/completions'),
    'chat.completion.async': ('POST', 'https://open.bigmodel.cn/api/paas/v4/async/chat/completions'),
    'video.generation.async': ('POST', 'https://open.bigmodel.cn/api/paas/v4/videos/generations'),
    'image.generation': ('POST', 'https://open.bigmodel.cn/api/paas/v4/images/generations'),
    'image.generation.async': ('POST', 'https://open.bigmodel.cn/api/paas/v4/async/images/generations'),
    'embedding': ('POST', 'https://open.bigmodel.cn/api/paas/v4/embeddings'),
    'audio.transcription': ('POST', 'https://open.bigmodel.cn/api/paas/v4/audio/transcriptions'),
    'audio.speech': ('POST', 'https://open.bigmodel.cn/api/paas/v4/audio/speech'),
    'audio.voice_clone': ('POST', 'https://open.bigmodel.cn/api/paas/v4/voice/clone'),
    'ocr': ('POST', 'https://open.bigmodel.cn/api/paas/v4/layout_parsing'),
    'realtime.websocket': (None, 'wss://open.bigmodel.cn/api/paas/v4/realtime'),
    'rerank': ('POST', 'https://open.bigmodel.cn/api/paas/v4/rerank'),
}

PathSpec = dict[str, Any]
Json = dict[str, Any]


def rust_string(value: str) -> str:
    return json.dumps(value, ensure_ascii=False)


def rust_option_string(value: str | None) -> str:
    return 'None' if not value else f'Some({rust_string(value)})'


def operation_expr(operation_id: str) -> str:
    return OPERATION_CONSTS.get(operation_id, f'OperationKind::new({rust_string(operation_id)})')


def surface_expr(surface_id: str) -> str:
    return SURFACE_CONSTS.get(surface_id, f'ApiSurfaceId::new({rust_string(surface_id)})')


def protocol_expr(protocol_id: str) -> str:
    return WIRE_PROTOCOL_CONSTS.get(protocol_id, f'WireProtocol::new({rust_string(protocol_id)})')


def provider_const_name(provider_id: str) -> str:
    return provider_id.upper().replace('-', '_')


def load_provider_catalog(path: Path) -> Json:
    return json.loads(path.read_text(encoding='utf-8'))


def all_provider_paths() -> list[Path]:
    return sorted(path for path in SOURCE_DIR.glob('*.json') if path.stem not in SKIP_PROVIDERS)


def collect_aliases(model_id: str, model: Json) -> list[str]:
    out: list[str] = []

    def add(value: Any) -> None:
        if isinstance(value, str) and value and value != model_id and value not in out:
            out.append(value)
        elif isinstance(value, list):
            for item in value:
                add(item)
        elif isinstance(value, dict):
            for item in value.values():
                add(item)

    for key in (
        'aliases',
        'api_alias',
        'canonical_slug',
        'ranking_permaslug',
        'model_code',
        'api_model_id',
        'vertex_model_id',
        'bedrock_model_id',
        'console_model_ids',
    ):
        add(model.get(key))

    for record in model.get('records', []):
        if record.get('table_kind') == 'alias_mapping':
            add(record.get('alias'))
        add(record.get('model_alias'))
        add(record.get('resolved_model'))
    return out


def model_candidates(model_id: str, model: Json) -> list[str]:
    candidates = [model_id]
    for alias in collect_aliases(model_id, model):
        if alias not in candidates:
            candidates.append(alias)
    return sorted(candidates, key=len, reverse=True)


def surface_to_operation(surface: str) -> str | None:
    return OPERATION_BY_SURFACE.get(surface)


def method_expr(method: str | None) -> str:
    mapping = {
        'GET': 'Some(HttpMethod::Get)',
        'POST': 'Some(HttpMethod::Post)',
        'PUT': 'Some(HttpMethod::Put)',
        'DELETE': 'Some(HttpMethod::Delete)',
    }
    return mapping.get((method or '').upper(), 'None')


def transport_expr(transport: str) -> str:
    return 'TransportKind::WebSocket' if transport == 'websocket' else 'TransportKind::Http'


def verification_expr(status: str) -> str:
    return f'VerificationStatus::{status}'


def provider_class_expr(provider_id: str) -> str:
    return PROVIDER_CLASS.get(provider_id, 'ProviderClass::Custom')


def auth_kinds(provider: Json) -> list[str]:
    auth = provider.get('auth') or {}
    out = AUTH_KIND_MAP.get(auth.get('type') or '', ['AuthMethodKind::ApiKeyHeader'])
    return out


def default_auth_header(provider_id: str) -> str | None:
    if provider_id == 'anthropic':
        return 'x-api-key'
    return 'authorization'


def default_auth_prefix(provider_id: str, auth_type: str) -> str | None:
    if auth_type == 'query_param_env':
        return None
    if provider_id == 'anthropic':
        return None
    return 'Bearer '


def auth_hint_expr(provider_id: str, provider: Json) -> str:
    auth = provider.get('auth') or {}
    auth_type = str(auth.get('type') or '').strip()
    keys = [str(key) for key in (auth.get('keys') or []) if str(key).strip()]

    if auth_type == 'query_param_env':
        param = str(auth.get('param') or 'key')
        return (
            'Some(ProviderAuthHint { '
            'method: AuthMethodKind::ApiKeyQuery, '
            f'env_keys: {render_string_slice(keys)}, '
            f'query_param: Some({rust_string(param)}), '
            'header_name: None, '
            'prefix: None, '
            '})'
        )

    if auth_type == 'api_key_env':
        header_name = default_auth_header(provider_id)
        prefix = default_auth_prefix(provider_id, auth_type)
        return (
            'Some(ProviderAuthHint { '
            'method: AuthMethodKind::ApiKeyHeader, '
            f'env_keys: {render_string_slice(keys)}, '
            'query_param: None, '
            f'header_name: {rust_option_string(header_name)}, '
            f'prefix: {rust_option_string(prefix)}, '
            '})'
        )

    return 'None'


def explicit_api_records(model: Json, surface: str) -> list[Json]:
    records = []
    for record in model.get('records', []):
        if record.get('table_kind') != 'api_reference':
            continue
        if record.get('api_surface') == surface:
            records.append(record)
    return records


def explicit_behavior_records(model: Json) -> list[Json]:
    records = []
    for record in model.get('records', []):
        if record.get('table_kind') == 'behavior':
            records.append(record)
    return records


def infer_wire_protocol(provider_id: str, surface: str, endpoint: str | None) -> str:
    endpoint = endpoint or ''
    parsed = urllib.parse.urlparse(endpoint)
    path = parsed.path
    host = parsed.netloc

    if surface in {'responses', 'response.create.beta'}:
        return 'openai.responses' if provider_id == 'openai' else 'ark.native' if provider_id == 'doubao' else 'openai.responses'
    if surface in {'completion.legacy', 'completion.fim.beta'}:
        return 'openai.text_completions'
    if surface == 'anthropic.messages':
        return 'anthropic.messages'
    if surface in {'chat.completion', 'group.chat.completion'}:
        if provider_id in {'deepseek', 'kimi', 'openrouter', 'xai'}:
            return 'openai.chat_completions'
        if provider_id == 'bailian':
            return 'openai.chat_completions'
        if provider_id == 'hunyuan':
            return 'hunyuan.native'
        if provider_id == 'qianfan':
            return 'qianfan.native'
        if provider_id == 'doubao':
            return 'ark.native'
        if provider_id == 'zhipu':
            return 'zhipu.native'
        if provider_id == 'minimax':
            return 'minimax.native' if 'chatcompletion_v2' in path else 'openai.chat_completions'
        if provider_id == 'google':
            return 'google.generate_content'
        if provider_id == 'anthropic':
            return 'anthropic.messages'
        if provider_id == 'openai':
            return 'openai.chat_completions'
        return 'openai.chat_completions' if 'chat/completions' in path else f'{provider_id}.native'
    if surface == 'generate.content':
        return 'google.generate_content'
    if surface == 'embedding':
        if provider_id in {'openai', 'deepseek'}:
            return 'openai.embeddings'
        if provider_id == 'google':
            return 'google.embed_content'
        if provider_id == 'bailian':
            return 'dashscope.native'
        if provider_id == 'qianfan':
            return 'qianfan.native'
        if provider_id == 'doubao':
            return 'ark.native'
        if provider_id == 'hunyuan':
            return 'hunyuan.native'
        if provider_id == 'zhipu':
            return 'zhipu.native'
        return 'openai.embeddings' if 'embeddings' in path else f'{provider_id}.native'
    if surface == 'embedding.multimodal':
        return 'qianfan.native' if provider_id == 'qianfan' else 'ark.native' if provider_id == 'doubao' else 'dashscope.native'
    if surface in {'image.generation', 'image.edit'}:
        if provider_id in {'openai', 'xai'}:
            return 'openai.images'
        if provider_id == 'google':
            return 'google.predict'
        if provider_id == 'bailian':
            return 'dashscope.native'
        if provider_id == 'qianfan':
            return 'qianfan.native'
        if provider_id == 'doubao':
            return 'ark.native'
        if provider_id == 'minimax':
            return 'minimax.native'
        if provider_id == 'zhipu':
            return 'zhipu.native'
        return f'{provider_id}.native'
    if surface in {'video.generation', 'video.generation.async', 'image.generation.async', '3d.generation'}:
        if provider_id == 'google':
            return 'google.predict_long_running'
        if provider_id == 'xai':
            return 'openai.images'
        if provider_id == 'qianfan':
            return 'qianfan.native'
        if provider_id == 'doubao':
            return 'ark.native'
        if provider_id == 'minimax':
            return 'minimax.native'
        if provider_id == 'zhipu':
            return 'zhipu.native'
        if provider_id == 'hunyuan':
            return 'hunyuan.native'
        if provider_id == 'bailian':
            return 'dashscope.native'
        return f'{provider_id}.native'
    if surface in {'audio.speech', 'audio.speech.async', 'audio.transcription', 'audio.transcription.realtime', 'audio.voice_clone', 'audio.voice_cloning', 'audio.voice_design'}:
        if provider_id == 'openai':
            return 'openai.audio'
        if provider_id == 'bailian':
            return 'dashscope.realtime_ws' if parsed.scheme.startswith('ws') and 'realtime' in path else 'dashscope.inference_ws' if parsed.scheme.startswith('ws') else 'dashscope.native'
        if provider_id == 'zhipu':
            return 'zhipu.native'
        if provider_id == 'minimax':
            return 'minimax.native'
        return f'{provider_id}.native'
    if surface == 'realtime.websocket':
        if provider_id == 'google':
            return 'google.live'
        if provider_id == 'openai':
            return 'openai.realtime'
        if provider_id == 'zhipu':
            return 'zhipu.native'
        if provider_id == 'bailian':
            return 'dashscope.realtime_ws'
        return f'{provider_id}.native'
    if surface in {'rerank', 'ocr', 'chat.translation', 'thread.run', 'classification_or_extraction', 'context.cache', 'batch', 'model.list', 'music.generation', 'image.question'}:
        if provider_id == 'qianfan':
            return 'qianfan.native'
        if provider_id == 'bailian':
            return 'dashscope.native'
        if provider_id == 'doubao':
            return 'ark.native'
        if provider_id == 'hunyuan':
            return 'hunyuan.native'
        if provider_id == 'minimax':
            return 'minimax.native'
        if provider_id == 'zhipu':
            return 'zhipu.native'
        return f'{provider_id}.native'
    return f'{provider_id}.native'


def fallback_endpoint(provider_id: str, surface: str) -> tuple[str | None, str | None]:
    if provider_id == 'doubao':
        return DOUBAO_ENDPOINTS.get(surface, (None, None))
    if provider_id == 'qianfan':
        return QIANFAN_ENDPOINTS.get(surface, (None, None))
    if provider_id == 'hunyuan':
        return HUNYUAN_ENDPOINTS.get(surface, (None, None))
    if provider_id == 'zhipu':
        return ZHIPU_ENDPOINTS.get(surface, (None, None))
    return None, None


def route_specs_for_surface(provider: Json, provider_id: str, model_id: str, model: Json, surface: str) -> list[PathSpec]:
    records = explicit_api_records(model, surface)
    method = None
    endpoint = None
    source_url = None
    record_base_url = None
    verification = 'Explicit'

    for record in records:
        endpoint = record.get('endpoint') or endpoint
        method = record.get('method') or method
        source_url = record.get('source_url') or source_url
        record_base_url = record.get('base_url') or record_base_url
        if endpoint:
            break

    if not endpoint:
        method, endpoint = fallback_endpoint(provider_id, surface)
        if not endpoint:
            return []
        source_url = source_url or provider.get('source_url')
        verification = 'FamilyInferred'

    operation_id = surface_to_operation(surface)
    if not operation_id:
        return []

    candidates = model_candidates(model_id, model)
    base_url = str(provider.get('base_url') or '')
    transport, http_method, base_override, path_template, query_params = normalize_endpoint(
        endpoint,
        method,
        base_url,
        candidates,
        explicit_base_url=record_base_url,
    )
    wire_protocol = infer_wire_protocol(provider_id, surface, endpoint)

    specs = [{
        'operation_id': operation_id,
        'surface_id': surface,
        'wire_protocol_id': wire_protocol,
        'transport': transport,
        'http_method': http_method,
        'base_url_override': base_override,
        'path_template': path_template,
        'query_params': query_params,
        'streaming': None,
        'async_job': True if surface.endswith('.async') else None,
        'verification': verification,
        'source_url': source_url or provider.get('source_url'),
    }]

    if provider_id == 'google' and surface == 'generate.content':
        specs.append({
            'operation_id': 'chat.completion',
            'surface_id': 'generate.content.stream',
            'wire_protocol_id': 'google.generate_content',
            'transport': 'http',
            'http_method': 'POST',
            'base_url_override': None,
            'path_template': '/v1beta/models/{model}:streamGenerateContent',
            'query_params': [('alt', 'sse')],
            'streaming': True,
            'async_job': None,
            'verification': 'FamilyInferred',
            'source_url': source_url or provider.get('source_url'),
        })
    return specs


def normalize_endpoint(
    endpoint: str,
    method: str | None,
    provider_base_url: str,
    candidates: list[str],
    explicit_base_url: str | None = None,
) -> tuple[str, str | None, str | None, str, list[tuple[str, str]]]:
    parsed = urllib.parse.urlparse(endpoint)
    transport = 'websocket' if parsed.scheme in {'ws', 'wss'} else 'http'
    http_method = None if transport == 'websocket' else (method or 'POST')

    provider_base = provider_base_url.rstrip('/')
    full_endpoint = endpoint.split('?', 1)[0]
    path_part = parsed.path or '/'
    query_params = urllib.parse.parse_qsl(parsed.query, keep_blank_values=True)

    if explicit_base_url:
        explicit_base = explicit_base_url.rstrip('/')
        base_override = None if explicit_base == provider_base else explicit_base
        path_template = path_part or '/'
    elif provider_base and full_endpoint.startswith(provider_base):
        base_override = None
        path_template = full_endpoint[len(provider_base):] or '/'
    else:
        base_override = f'{parsed.scheme}://{parsed.netloc}' if parsed.scheme and parsed.netloc else None
        path_template = path_part or '/'

    path_template = templatize_value(path_template, candidates)
    normalized_query = [(name, templatize_value(value, candidates)) for name, value in query_params]
    return transport, http_method, base_override, path_template or '/', normalized_query


def templatize_value(value: str, candidates: list[str]) -> str:
    out = value
    for candidate in candidates:
        for encoded in {
            candidate,
            urllib.parse.quote(candidate, safe=':/@-._~'),
            urllib.parse.quote(candidate, safe=''),
        }:
            if encoded and encoded in out:
                out = out.replace(encoded, '{model}')
    return out


def collect_bindings(provider: Json, provider_id: str, models: Json) -> list[dict[str, Any]]:
    grouped: dict[tuple[Any, ...], dict[str, Any]] = {}
    for model_id, model in models.items():
        surfaces = list(model.get('api_surfaces') or [])
        for surface in surfaces:
            for spec in route_specs_for_surface(provider, provider_id, model_id, model, surface):
                key = (
                    spec['operation_id'],
                    spec['surface_id'],
                    spec['wire_protocol_id'],
                    spec['transport'],
                    spec['http_method'],
                    spec['base_url_override'],
                    spec['path_template'],
                    tuple(spec['query_params']),
                    spec['streaming'],
                    spec['async_job'],
                    spec['verification'],
                )
                bucket = grouped.setdefault(
                    key,
                    {
                        **spec,
                        'models': [],
                    },
                )
                bucket['models'].append(model_id)
    return sorted(grouped.values(), key=lambda item: (item['operation_id'], item['surface_id'], item['path_template']))


def model_brand(provider_id: str, model: Json) -> str | None:
    for key in ('brand', 'vendor'):
        value = model.get(key)
        if isinstance(value, str) and value:
            return value
    return provider_id if provider_id in {'anthropic', 'google', 'kimi', 'openai', 'openrouter', 'xai', 'deepseek'} else None


def model_family(provider_id: str, model_id: str, model: Json) -> str | None:
    brand = model_brand(provider_id, model)
    if brand:
        return brand
    if '-' in model_id:
        return model_id.split('-', 1)[0]
    return None


def supported_operations(model: Json) -> list[str]:
    out: list[str] = []
    for surface in model.get('api_surfaces') or []:
        operation = surface_to_operation(surface)
        if operation and operation not in out:
            out.append(operation)
    return out


def render_query_params(query_params: list[tuple[str, str]]) -> str:
    if not query_params:
        return '&[]'
    parts = []
    for name, value in query_params:
        parts.append(f'EndpointQueryParam {{ name: {rust_string(name)}, value_template: {rust_string(value)} }}')
    return '&[' + ', '.join(parts) + ']'


def render_string_slice(values: list[str]) -> str:
    if not values:
        return '&[]'
    return '&[' + ', '.join(rust_string(value) for value in values) + ']'


def render_operation_slice(values: list[str]) -> str:
    if not values:
        return '&[]'
    return '&[' + ', '.join(operation_expr(value) for value in values) + ']'


def capability_expr(operation_id: str) -> str | None:
    return CAPABILITY_STATUS_CONSTS.get(operation_id)


def render_capability_status_slice(operation_ids: list[str]) -> str:
    capabilities: list[str] = []
    for operation_id in operation_ids:
        capability = capability_expr(operation_id)
        if capability and capability not in capabilities:
            capabilities.append(capability)
    if not capabilities:
        return '&[]'
    return '&[' + ', '.join(
        f'CapabilityStatusDescriptor::implemented({capability})'
        for capability in capabilities
    ) + ']'


def behavior_support_expr(value: str | None) -> str:
    return BEHAVIOR_SUPPORT_CONSTS.get((value or 'unknown').strip(), 'BehaviorSupport::Unknown')


def assistant_tool_followup_expr(value: str | None) -> str:
    return ASSISTANT_TOOL_FOLLOWUP_CONSTS.get(
        (value or 'none').strip(),
        'AssistantToolFollowupRequirement::None',
    )


def reasoning_output_expr(value: str | None) -> str:
    return REASONING_OUTPUT_CONSTS.get(
        (value or 'unsupported').strip(),
        'ReasoningOutputMode::Unsupported',
    )


def reasoning_activation_expr(value: str | None) -> str:
    return REASONING_ACTIVATION_CONSTS.get(
        (value or 'unavailable').strip(),
        'ReasoningActivationKind::Unavailable',
    )


def cache_usage_reporting_expr(value: str | None) -> str:
    return CACHE_USAGE_REPORTING_CONSTS.get(
        (value or 'unknown').strip(),
        'CacheUsageReportingKind::Unknown',
    )


def render_context_cache_modes(values: list[str]) -> str:
    if not values:
        return '&[]'
    rendered = [
        CONTEXT_CACHE_MODE_CONSTS.get(value, f'ContextCacheModeId::new({rust_string(value)})')
        for value in values
    ]
    return '&[' + ', '.join(rendered) + ']'


def collect_behaviors(models: Json) -> list[dict[str, Any]]:
    entries: list[dict[str, Any]] = []
    for model_id, model in models.items():
        for record in explicit_behavior_records(model):
            operation_id = str(record.get('operation') or '').strip()
            if not operation_id:
                continue
            entries.append({
                'model': model_id,
                'operation_id': operation_id,
                'tool_calls': str(record.get('tool_calls') or 'unknown'),
                'tool_choice_required': str(record.get('tool_choice_required') or 'unknown'),
                'assistant_tool_followup': str(record.get('assistant_tool_followup') or 'none'),
                'reasoning_output': str(record.get('reasoning_output') or 'unsupported'),
                'reasoning_activation': str(record.get('reasoning_activation') or 'unavailable'),
                'context_cache_modes': [str(value) for value in (record.get('context_cache_modes') or [])],
                'context_cache_default_enabled': bool(record.get('context_cache_default_enabled') or False),
                'cache_usage_reporting': str(record.get('cache_usage_reporting') or 'unknown'),
                'notes': record.get('notes'),
            })
    return sorted(entries, key=lambda item: (item['model'], item['operation_id']))


def render_model_descriptors(provider_id: str, models: Json) -> str:
    entries = []
    for model_id in sorted(models):
        model = models[model_id]
        aliases = collect_aliases(model_id, model)
        display_name = str(model.get('display_name') or model_id)
        summary = model.get('summary') or model.get('description')
        operations = supported_operations(model)
        entries.append(
            '    ProviderModelDescriptor {\n'
            f'        id: {rust_string(model_id)},\n'
            f'        display_name: {rust_string(display_name)},\n'
            f'        aliases: {render_string_slice(aliases)},\n'
            f'        brand: {rust_option_string(model_brand(provider_id, model))},\n'
            f'        family: {rust_option_string(model_family(provider_id, model_id, model))},\n'
            f'        summary: {rust_option_string(summary if isinstance(summary, str) and summary else None)},\n'
            f'        supported_operations: {render_operation_slice(operations)},\n'
            f'        capability_statuses: {render_capability_status_slice(operations)},\n'
            '    }'
        )
    return '&[\n' + ',\n'.join(entries) + '\n]'


def render_bindings(bindings: list[dict[str, Any]], evidence_const: str) -> str:
    entries = []
    for binding in bindings:
        entries.append(
            '    ModelBinding {\n'
            f'        operation: {operation_expr(binding["operation_id"])},\n'
            f'        selector: ModelSelector::Exact({render_string_slice(sorted(binding["models"]))}),\n'
            f'        surface: {surface_expr(binding["surface_id"])},\n'
            f'        wire_protocol: {protocol_expr(binding["wire_protocol_id"])},\n'
            '        endpoint: EndpointTemplate {\n'
            f'            transport: {transport_expr(binding["transport"])},\n'
            f'            http_method: {method_expr(binding["http_method"])},\n'
            f'            base_url_override: {rust_option_string(binding["base_url_override"])},\n'
            f'            path_template: {rust_string(binding["path_template"])},\n'
            f'            query_params: {render_query_params(binding["query_params"])},\n'
            '        },\n'
            '        quirks: None,\n'
            f'        streaming: {"Some(true)" if binding["streaming"] is True else "Some(false)" if binding["streaming"] is False else "None"},\n'
            f'        async_job: {"Some(true)" if binding["async_job"] is True else "Some(false)" if binding["async_job"] is False else "None"},\n'
            f'        verification: {verification_expr(binding["verification"])},\n'
            f'        evidence: {evidence_const},\n'
            '    }'
        )
    return '&[\n' + ',\n'.join(entries) + '\n]'


def render_behaviors(behaviors: list[dict[str, Any]]) -> str:
    if not behaviors:
        return '&[]'
    entries = []
    for behavior in behaviors:
        entries.append(
            '    ModelBehaviorDescriptor {\n'
            f'        model: {rust_string(behavior["model"])},\n'
            f'        operation: {operation_expr(behavior["operation_id"])},\n'
            f'        tool_calls: {behavior_support_expr(behavior["tool_calls"])},\n'
            f'        tool_choice_required: {behavior_support_expr(behavior["tool_choice_required"])},\n'
            f'        assistant_tool_followup: {assistant_tool_followup_expr(behavior["assistant_tool_followup"])},\n'
            f'        reasoning_output: {reasoning_output_expr(behavior["reasoning_output"])},\n'
            f'        reasoning_activation: {reasoning_activation_expr(behavior["reasoning_activation"])},\n'
            f'        context_cache_modes: {render_context_cache_modes(behavior["context_cache_modes"])},\n'
            f'        context_cache_default_enabled: {"true" if behavior["context_cache_default_enabled"] else "false"},\n'
            f'        cache_usage_reporting: {cache_usage_reporting_expr(behavior["cache_usage_reporting"])},\n'
            f'        notes: {rust_option_string(behavior["notes"] if isinstance(behavior["notes"], str) and behavior["notes"] else None)},\n'
            '    }'
        )
    return '&[\n' + ',\n'.join(entries) + '\n]'


def render_prelude() -> str:
    return '\n'.join([
        '// Generated by scripts/generate_rust_provider_catalog.py. Do not edit by hand.',
        '#![allow(unused_imports)]',
        'use crate::catalog::{',
        '    ApiSurfaceId, AssistantToolFollowupRequirement, AuthMethodKind, BehaviorSupport,',
        '    CacheUsageReportingKind, CapabilityKind, CapabilityStatusDescriptor, ContextCacheModeId,',
        '    EndpointQueryParam, EndpointTemplate, EvidenceLevel, EvidenceRef, HttpMethod,',
        '    ModelBehaviorDescriptor, ModelBinding, ModelSelector, OperationKind, ProtocolQuirks,',
        '    ProviderAuthHint, ProviderClass, ProviderModelDescriptor, ProviderPluginDescriptor,',
        '    ReasoningActivationKind, ReasoningOutputMode, TransportKind, VerificationStatus,',
        '    WireProtocol,',
        '};',
        '',
    ])


def render_expr_slice(values: list[str], indent: str = '') -> str:
    if not values:
        return '&[]'
    inner_indent = indent + '    '
    return '&[\n' + ''.join(f'{inner_indent}{value},\n' for value in values) + indent + ']'


def render_array_literal(values: list[str], indent: str = '') -> str:
    if not values:
        return '[]'
    inner_indent = indent + '    '
    return '[\n' + ''.join(f'{inner_indent}{value},\n' for value in values) + indent + ']'


def render_static_id_impl(type_name: str, specs: list[tuple[str, str]]) -> str:
    lines = [f'impl {type_name} {{']
    for const_name, value in specs:
        lines.append(f'    pub const {const_name}: Self = Self::new({rust_string(value)});')
    lines.append('}')
    return '\n'.join(lines)


def render_capability_impl() -> str:
    lines = [
        'impl CapabilityKind {',
        "    pub const fn new(id: &'static str) -> Self {",
        '        Self(id)',
        '    }',
        '',
        "    pub const fn as_str(self) -> &'static str {",
        '        self.0',
        '    }',
        '',
        '    pub fn parse_config_token(value: &str) -> Option<Self> {',
        '        match value.trim().to_ascii_lowercase().as_str() {',
    ]
    for const_name, _, aliases, _ in CAPABILITY_SPECS:
        patterns = ' | '.join(rust_string(alias) for alias in aliases)
        lines.append(f'            {patterns} => Some(Self::{const_name}),')
    lines.extend([
        '            _ => None,',
        '        }',
        '    }',
        '',
    ])
    for const_name, value, _, _ in CAPABILITY_SPECS:
        lines.append(f'    pub const {const_name}: Self = Self::new({rust_string(value)});')
    lines.append('}')
    return '\n'.join(lines)


def render_capability_for_operation() -> str:
    grouped: dict[str, list[str]] = defaultdict(list)
    for const_name, _, routing_capability_const, _ in OPERATION_SPECS:
        grouped[routing_capability_const].append(f'OperationKind::{const_name}')

    lines = [
        'pub fn capability_for_operation(operation: OperationKind) -> Option<CapabilityKind> {',
        '    match operation {',
    ]
    for capability_const, _, _, _ in CAPABILITY_SPECS:
        operations = grouped.get(capability_const)
        if not operations:
            continue
        lines.append(
            f'        {" | ".join(operations)} => Some(CapabilityKind::{capability_const}),'
        )
    lines.extend([
        '        _ => None,',
        '    }',
        '}',
    ])
    return '\n'.join(lines)


def render_invocation_probe_consts() -> str:
    lines: list[str] = []
    for capability_const, _, _, probe_operations in CAPABILITY_SPECS:
        if not probe_operations:
            continue
        probe_exprs = [operation_expr(operation_id) for operation_id in probe_operations]
        lines.append(
            f'const {capability_const}_INVOCATION_OPERATIONS: &[OperationKind] = '
            f'{render_expr_slice(probe_exprs)};'
        )
    return '\n'.join(lines)


def render_invocation_operations_for_capability() -> str:
    lines = [
        '/// CONTRACT-CAPABILITY-INVOCATION-OPS: ordered generic invocation operations',
        '/// probed by runtime builders for a capability adapter.',
        '///',
        '/// This is intentionally not the exhaustive inverse of `capability_for_operation`.',
        '/// It only describes the stable invocation surfaces that the generic runtime',
        '/// builders can assemble today.',
        '///',
        '/// `None` means the capability is known to the contracts layer, but generic',
        '/// runtime builders do not probe invocation routes for it.',
        'pub fn invocation_operations_for_capability(',
        '    capability: CapabilityKind,',
        ") -> Option<&'static [OperationKind]> {",
        '    match capability {',
    ]
    for capability_const, _, _, probe_operations in CAPABILITY_SPECS:
        if not probe_operations:
            continue
        lines.append(
            f'        CapabilityKind::{capability_const} => Some({capability_const}_INVOCATION_OPERATIONS),'
        )
    lines.extend([
        '        _ => None,',
        '    }',
        '}',
    ])
    return '\n'.join(lines)


def render_ids_tests() -> str:
    llm_probe_operations = next(
        probe_operations
        for const_name, _, _, probe_operations in CAPABILITY_SPECS
        if const_name == 'LLM'
    )
    llm_probe_exprs = [operation_expr(operation_id) for operation_id in llm_probe_operations]
    operation_mappings = [
        (f'OperationKind::{const_name}', f'CapabilityKind::{routing_capability_const}')
        for const_name, _, routing_capability_const, _ in OPERATION_SPECS
    ]
    no_probe_capabilities = [
        f'CapabilityKind::{const_name}'
        for const_name, _, _, probe_operations in CAPABILITY_SPECS
        if not probe_operations
    ]
    lines = [
        '#[cfg(test)]',
        'mod tests {',
        '    use super::{',
        '        CapabilityKind, OperationKind, capability_for_operation,',
        '        invocation_operations_for_capability,',
        '    };',
        '',
        '    #[test]',
        '    fn named_operations_map_to_expected_capabilities() {',
        '        for (operation, capability) in [',
    ]
    for operation, capability in operation_mappings:
        lines.append(f'            ({operation}, {capability}),')
    lines.extend([
        '        ] {',
        '            assert_eq!(capability_for_operation(operation), Some(capability));',
        '        }',
        '    }',
        '',
        '    #[test]',
        '    fn llm_invocation_operations_cover_generic_text_surfaces() {',
        '        assert_eq!(',
        '            invocation_operations_for_capability(CapabilityKind::LLM),',
        f'            Some(&{render_array_literal(llm_probe_exprs, "            ")}[..])',
        '        );',
        '    }',
        '',
        '    #[test]',
        '    fn batch_and_rerank_invocation_operations_are_single_surface() {',
        '        assert_eq!(',
        '            invocation_operations_for_capability(CapabilityKind::BATCH),',
        f'            Some(&{render_array_literal([operation_expr("batch")], "            ")}[..])',
        '        );',
        '        assert_eq!(',
        '            invocation_operations_for_capability(CapabilityKind::RERANK),',
        f'            Some(&{render_array_literal([operation_expr("rerank")], "            ")}[..])',
        '        );',
        '    }',
        '',
        '    #[test]',
        '    fn non_generic_builder_capabilities_have_no_probe_operations() {',
        '        for capability in [',
    ])
    for capability in no_probe_capabilities:
        lines.append(f'            {capability},')
    lines.extend([
        '        ] {',
        '            assert_eq!(invocation_operations_for_capability(capability), None);',
        '        }',
        '    }',
        '}',
    ])
    return '\n'.join(lines)


def render_ids_file() -> str:
    operation_impl = render_static_id_impl(
        'OperationKind', [(const_name, operation_id) for const_name, operation_id, _, _ in OPERATION_SPECS]
    )
    surface_impl = render_static_id_impl('ApiSurfaceId', SURFACE_SPECS)
    wire_protocol_impl = render_static_id_impl('WireProtocol', WIRE_PROTOCOL_SPECS)
    context_cache_mode_impl = render_static_id_impl('ContextCacheModeId', CONTEXT_CACHE_MODE_SPECS)
    return '\n'.join([
        '// Generated by scripts/generate_rust_provider_catalog.py. Do not edit by hand.',
        'use core::fmt;',
        '',
        'macro_rules! static_id_type {',
        '    ($name:ident) => {',
        '        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]',
        "        pub struct $name(&'static str);",
        '',
        '        impl $name {',
        "            pub const fn new(id: &'static str) -> Self {",
        '                Self(id)',
        '            }',
        '',
        "            pub const fn as_str(self) -> &'static str {",
        '                self.0',
        '            }',
        '        }',
        '',
        '        impl fmt::Display for $name {',
        "            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {",
        '                f.write_str(self.0)',
        '            }',
        '        }',
        '    };',
        '}',
        '',
        'static_id_type!(OperationKind);',
        'static_id_type!(ApiSurfaceId);',
        'static_id_type!(WireProtocol);',
        'static_id_type!(ContextCacheModeId);',
        '',
        operation_impl,
        '',
        surface_impl,
        '',
        wire_protocol_impl,
        '',
        context_cache_mode_impl,
        '',
        '#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]',
        "pub struct ProviderId<'a>(&'a str);",
        '',
        "impl<'a> ProviderId<'a> {",
        "    pub const fn new(id: &'a str) -> Self {",
        '        Self(id)',
        '    }',
        '',
        "    pub const fn as_str(self) -> &'a str {",
        '        self.0',
        '    }',
        '}',
        '',
        "impl fmt::Display for ProviderId<'_> {",
        "    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {",
        '        f.write_str(self.0)',
        '    }',
        '}',
        '',
        "impl<'a> From<&'a str> for ProviderId<'a> {",
        "    fn from(value: &'a str) -> Self {",
        '        Self::new(value)',
        '    }',
        '}',
        '',
        '#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]',
        "pub struct CapabilityKind(&'static str);",
        '',
        render_capability_impl(),
        '',
        'impl fmt::Display for CapabilityKind {',
        "    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {",
        '        f.write_str(self.0)',
        '    }',
        '}',
        '',
        render_capability_for_operation(),
        '',
        render_invocation_probe_consts(),
        '',
        render_invocation_operations_for_capability(),
        '',
        render_ids_tests(),
        '',
    ])


def provider_cfg_attr(provider_id: str) -> str:
    return f'#[cfg(feature = {rust_string(PROVIDER_FEATURES[provider_id])})]'


def provider_module_name(provider_id: str) -> str:
    return provider_id.replace('-', '_')


def render_provider(provider_id: str, data: Json) -> str:
    provider = data['provider']
    models = data.get('models') or {}
    bindings = collect_bindings(provider, provider_id, models)
    behaviors = collect_behaviors(models)
    provider_operations: list[str] = []
    for binding in bindings:
        operation_id = str(binding['operation_id'])
        if operation_id not in provider_operations:
            provider_operations.append(operation_id)
    for model in models.values():
        for operation_id in supported_operations(model):
            if operation_id not in provider_operations:
                provider_operations.append(operation_id)
    const_prefix = provider_const_name(provider_id)
    feature = PROVIDER_FEATURES[provider_id]
    evidence_const = f'{const_prefix}_EVIDENCE'
    models_const = f'{const_prefix}_MODELS'
    bindings_const = f'{const_prefix}_BINDINGS'
    behaviors_const = f'{const_prefix}_BEHAVIORS'
    plugin_const = f'{const_prefix}_PLUGIN'
    auth_values = ', '.join(auth_kinds(provider))
    provider_source_url = str(provider.get('source_url') or '')

    if provider_id == 'openai':
        return '\n'.join([
            f'#[cfg(feature = {rust_string(feature)})]',
            f'pub(crate) const {models_const}: &[ProviderModelDescriptor] = {render_model_descriptors(provider_id, models)};',
            '',
            f'#[cfg(feature = {rust_string(feature)})]',
            f'pub(crate) const {behaviors_const}: &[ModelBehaviorDescriptor] = {render_behaviors(behaviors)};',
        ])

    return '\n'.join([
        f'#[cfg(feature = {rust_string(feature)})]',
        f'pub(crate) const {evidence_const}: &[EvidenceRef] = &[EvidenceRef {{',
        '    level: EvidenceLevel::OfficialDocs,',
        f'    source_url: {rust_string(provider_source_url)},',
        f'    note: Some({rust_string("Generated from provider reference catalog and compiled into Rust.")}),',
        '}];',
        '',
        f'#[cfg(feature = {rust_string(feature)})]',
        f'pub(crate) const {models_const}: &[ProviderModelDescriptor] = {render_model_descriptors(provider_id, models)};',
        '',
        f'#[cfg(feature = {rust_string(feature)})]',
        f'pub(crate) const {bindings_const}: &[ModelBinding] = {render_bindings(bindings, evidence_const)};',
        '',
        f'#[cfg(feature = {rust_string(feature)})]',
        f'pub(crate) const {behaviors_const}: &[ModelBehaviorDescriptor] = {render_behaviors(behaviors)};',
        '',
        f'#[cfg(feature = {rust_string(feature)})]',
        f'pub const {plugin_const}: ProviderPluginDescriptor = ProviderPluginDescriptor {{',
        f'    id: {rust_string(provider_id)},',
        f'    display_name: {rust_string(str(provider.get("display_name") or provider_id))},',
        f'    class: {provider_class_expr(provider_id)},',
        f'    default_base_url: {rust_option_string(str(provider.get("base_url") or "") or None)},',
        f'    supported_auth: &[{auth_values}],',
        f'    auth_hint: {auth_hint_expr(provider_id, provider)},',
        f'    models: {models_const},',
        f'    bindings: {bindings_const},',
        f'    behaviors: {behaviors_const},',
        f'    capability_statuses: {render_capability_status_slice(provider_operations)},',
        '};',
    ])


def rendered_provider_catalogs() -> list[tuple[str, str]]:
    catalogs: list[tuple[str, str]] = []
    for path in all_provider_paths():
        data = load_provider_catalog(path)
        provider_id = data['provider']['id']
        if provider_id not in PROVIDER_FEATURES:
            continue
        catalogs.append((provider_id, render_provider(provider_id, data)))
    return catalogs


def render_provider_file(provider_block: str) -> str:
    return render_prelude() + provider_block + '\n'


def render_providers_mod(provider_ids: list[str]) -> str:
    lines = [
        '// Generated by scripts/generate_rust_provider_catalog.py. Do not edit by hand.',
        '#![allow(unused_imports)]',
        '',
    ]
    for provider_id in provider_ids:
        cfg_attr = provider_cfg_attr(provider_id)
        module_name = provider_module_name(provider_id)
        lines.extend([
            cfg_attr,
            f'mod {module_name};',
            cfg_attr,
            f'pub(crate) use {module_name}::*;',
            '',
        ])
    return '\n'.join(lines).rstrip() + '\n'


def write_text(path: Path, content: str) -> None:
    if path.exists() and path.read_text(encoding='utf-8') == content:
        return
    path.write_text(content, encoding='utf-8')


def remove_stale_provider_files(active_module_names: set[str]) -> None:
    if not TARGET_MODULE_DIR.exists():
        return
    for path in TARGET_MODULE_DIR.glob('*.rs'):
        if path.name == 'mod.rs':
            continue
        if path.stem not in active_module_names:
            path.unlink()


def main() -> int:
    TARGET_DIR.mkdir(parents=True, exist_ok=True)
    TARGET_MODULE_DIR.mkdir(parents=True, exist_ok=True)

    write_text(CONTRACT_IDS_TARGET_FILE, render_ids_file())
    print(CONTRACT_IDS_TARGET_FILE.relative_to(ROOT))

    catalogs = rendered_provider_catalogs()
    active_module_names: set[str] = set()
    for provider_id, provider_block in catalogs:
        module_name = provider_module_name(provider_id)
        active_module_names.add(module_name)
        target_file = TARGET_MODULE_DIR / f'{module_name}.rs'
        write_text(target_file, render_provider_file(provider_block))
        print(target_file.relative_to(ROOT))

    write_text(TARGET_MODULE_DIR / 'mod.rs', render_providers_mod([provider_id for provider_id, _ in catalogs]))
    print((TARGET_MODULE_DIR / 'mod.rs').relative_to(ROOT))

    remove_stale_provider_files(active_module_names)

    if LEGACY_TARGET_FILE.exists():
        LEGACY_TARGET_FILE.unlink()
        print(LEGACY_TARGET_FILE.relative_to(ROOT))

    return 0


if __name__ == '__main__':
    raise SystemExit(main())
