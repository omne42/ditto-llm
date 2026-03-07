#!/usr/bin/env python3
from __future__ import annotations

import json
import urllib.parse
from collections import defaultdict
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
SOURCE_DIR = ROOT / 'catalog' / 'provider_models'
TARGET_DIR = ROOT / 'src' / 'catalog' / 'generated'
TARGET_FILE = TARGET_DIR / 'providers.rs'

SKIP_PROVIDERS: set[str] = set()

PROVIDER_FEATURES = {
    'openai': 'openai',
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

OPERATION_CONSTS = {
    'response': 'OperationKind::RESPONSE',
    'chat.completion': 'OperationKind::CHAT_COMPLETION',
    'group.chat.completion': 'OperationKind::GROUP_CHAT_COMPLETION',
    'text.completion': 'OperationKind::TEXT_COMPLETION',
    'embedding': 'OperationKind::EMBEDDING',
    'embedding.multimodal': 'OperationKind::MULTIMODAL_EMBEDDING',
    'image.generation': 'OperationKind::IMAGE_GENERATION',
    'image.edit': 'OperationKind::IMAGE_EDIT',
    'image.translation': 'OperationKind::IMAGE_TRANSLATION',
    'image.question': 'OperationKind::IMAGE_QUESTION',
    'video.generation': 'OperationKind::VIDEO_GENERATION',
    'audio.speech': 'OperationKind::AUDIO_SPEECH',
    'audio.transcription': 'OperationKind::AUDIO_TRANSCRIPTION',
    'audio.translation': 'OperationKind::AUDIO_TRANSLATION',
    'audio.voice_clone': 'OperationKind::AUDIO_VOICE_CLONE',
    'audio.voice_design': 'OperationKind::AUDIO_VOICE_DESIGN',
    'music.generation': 'OperationKind::MUSIC_GENERATION',
    'rerank': 'OperationKind::RERANK',
    'classification_or_extraction': 'OperationKind::CLASSIFICATION_OR_EXTRACTION',
    'moderation': 'OperationKind::MODERATION',
    'batch': 'OperationKind::BATCH',
    'ocr': 'OperationKind::OCR',
    'realtime.session': 'OperationKind::REALTIME_SESSION',
    'thread.run': 'OperationKind::THREAD_RUN',
    'chat.translation': 'OperationKind::CHAT_TRANSLATION',
    'context.cache': 'OperationKind::CONTEXT_CACHE',
    'model.list': 'OperationKind::MODEL_LIST',
    '3d.generation': 'OperationKind::THREE_D_GENERATION',
}

SURFACE_CONSTS = {
    'chat.completion': 'ApiSurfaceId::OPENAI_CHAT_COMPLETIONS',
    'responses': 'ApiSurfaceId::OPENAI_RESPONSES',
    'completion.legacy': 'ApiSurfaceId::OPENAI_TEXT_COMPLETIONS',
    'embedding': 'ApiSurfaceId::OPENAI_EMBEDDINGS',
    'image.generation': 'ApiSurfaceId::OPENAI_IMAGES_GENERATIONS',
    'image.edit': 'ApiSurfaceId::OPENAI_IMAGES_EDITS',
    'audio.speech': 'ApiSurfaceId::OPENAI_AUDIO_SPEECH',
    'audio.transcription': 'ApiSurfaceId::OPENAI_AUDIO_TRANSCRIPTIONS',
    'audio.translation': 'ApiSurfaceId::OPENAI_AUDIO_TRANSLATIONS',
    'moderation': 'ApiSurfaceId::OPENAI_MODERATIONS',
    'batch': 'ApiSurfaceId::OPENAI_BATCHES',
    'realtime.websocket': 'ApiSurfaceId::OPENAI_REALTIME',
    'anthropic.messages': 'ApiSurfaceId::ANTHROPIC_MESSAGES',
    'generate.content': 'ApiSurfaceId::GOOGLE_GENERATE_CONTENT',
    'generate.content.stream': 'ApiSurfaceId::GOOGLE_STREAM_GENERATE_CONTENT',
    'generate.content.batch': 'ApiSurfaceId::GOOGLE_BATCH_GENERATE_CONTENT',
}

WIRE_PROTOCOL_CONSTS = {
    'openai.chat_completions': 'WireProtocol::OPENAI_CHAT_COMPLETIONS',
    'openai.responses': 'WireProtocol::OPENAI_RESPONSES',
    'openai.text_completions': 'WireProtocol::OPENAI_TEXT_COMPLETIONS',
    'openai.embeddings': 'WireProtocol::OPENAI_EMBEDDINGS',
    'openai.images': 'WireProtocol::OPENAI_IMAGES',
    'openai.audio': 'WireProtocol::OPENAI_AUDIO',
    'openai.moderations': 'WireProtocol::OPENAI_MODERATIONS',
    'openai.batches': 'WireProtocol::OPENAI_BATCHES',
    'openai.realtime': 'WireProtocol::OPENAI_REALTIME',
    'anthropic.messages': 'WireProtocol::ANTHROPIC_MESSAGES',
    'google.generate_content': 'WireProtocol::GOOGLE_GENERATE_CONTENT',
    'google.embed_content': 'WireProtocol::GOOGLE_EMBED_CONTENT',
    'google.live': 'WireProtocol::GOOGLE_LIVE',
    'google.predict': 'WireProtocol::GOOGLE_PREDICT',
    'google.predict_long_running': 'WireProtocol::GOOGLE_PREDICT_LONG_RUNNING',
    'dashscope.native': 'WireProtocol::DASHSCOPE_NATIVE',
    'dashscope.inference_ws': 'WireProtocol::DASHSCOPE_INFERENCE_WS',
    'dashscope.realtime_ws': 'WireProtocol::DASHSCOPE_REALTIME_WS',
    'qianfan.native': 'WireProtocol::QIANFAN_NATIVE',
    'ark.native': 'WireProtocol::ARK_NATIVE',
    'hunyuan.native': 'WireProtocol::HUNYUAN_NATIVE',
    'minimax.native': 'WireProtocol::MINIMAX_NATIVE',
    'zhipu.native': 'WireProtocol::ZHIPU_NATIVE',
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
    verification = 'Explicit'

    for record in records:
        endpoint = record.get('endpoint') or endpoint
        method = record.get('method') or method
        source_url = record.get('source_url') or source_url
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


def normalize_endpoint(endpoint: str, method: str | None, provider_base_url: str, candidates: list[str]) -> tuple[str, str | None, str | None, str, list[tuple[str, str]]]:
    parsed = urllib.parse.urlparse(endpoint)
    transport = 'websocket' if parsed.scheme in {'ws', 'wss'} else 'http'
    http_method = None if transport == 'websocket' else (method or 'POST')

    base_url = provider_base_url.rstrip('/')
    full_endpoint = endpoint.split('?', 1)[0]
    path_part = parsed.path or '/'
    query_params = urllib.parse.parse_qsl(parsed.query, keep_blank_values=True)

    if base_url and full_endpoint.startswith(base_url):
        base_override = None
        path_template = full_endpoint[len(base_url):] or '/'
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


def render_model_descriptors(provider_id: str, models: Json) -> str:
    entries = []
    for model_id in sorted(models):
        model = models[model_id]
        aliases = collect_aliases(model_id, model)
        display_name = str(model.get('display_name') or model_id)
        summary = model.get('summary') or model.get('description')
        entries.append(
            '    ProviderModelDescriptor {\n'
            f'        id: {rust_string(model_id)},\n'
            f'        display_name: {rust_string(display_name)},\n'
            f'        aliases: {render_string_slice(aliases)},\n'
            f'        brand: {rust_option_string(model_brand(provider_id, model))},\n'
            f'        family: {rust_option_string(model_family(provider_id, model_id, model))},\n'
            f'        summary: {rust_option_string(summary if isinstance(summary, str) and summary else None)},\n'
            f'        supported_operations: {render_operation_slice(supported_operations(model))},\n'
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


def render_provider(provider_id: str, data: Json) -> str:
    provider = data['provider']
    models = data.get('models') or {}
    bindings = collect_bindings(provider, provider_id, models)
    const_prefix = provider_const_name(provider_id)
    feature = PROVIDER_FEATURES[provider_id]
    evidence_const = f'{const_prefix}_EVIDENCE'
    models_const = f'{const_prefix}_MODELS'
    bindings_const = f'{const_prefix}_BINDINGS'
    plugin_const = f'{const_prefix}_PLUGIN'
    auth_values = ', '.join(auth_kinds(provider))
    provider_source_url = str(provider.get('source_url') or '')

    if provider_id == 'openai':
        return '\n'.join([
            f'#[cfg(feature = {rust_string(feature)})]',
            f'pub(crate) const {models_const}: &[ProviderModelDescriptor] = {render_model_descriptors(provider_id, models)};',
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
        f'pub const {plugin_const}: ProviderPluginDescriptor = ProviderPluginDescriptor {{',
        f'    id: {rust_string(provider_id)},',
        f'    display_name: {rust_string(str(provider.get("display_name") or provider_id))},',
        f'    class: {provider_class_expr(provider_id)},',
        f'    default_base_url: {rust_option_string(str(provider.get("base_url") or "") or None)},',
        f'    supported_auth: &[{auth_values}],',
        f'    auth_hint: {auth_hint_expr(provider_id, provider)},',
        f'    models: {models_const},',
        f'    bindings: {bindings_const},',
        '};',
    ])


def render_file() -> str:
    blocks = []
    for path in all_provider_paths():
        data = load_provider_catalog(path)
        provider_id = data['provider']['id']
        if provider_id not in PROVIDER_FEATURES:
            continue
        blocks.append(render_provider(provider_id, data))

    prelude = '\n'.join([
        '// Generated by scripts/generate_rust_provider_catalog.py. Do not edit by hand.',
        '#![allow(unused_imports)]',
        'use crate::catalog::{',
        '    ApiSurfaceId, AuthMethodKind, EndpointQueryParam, EndpointTemplate, EvidenceLevel,',
        '    EvidenceRef, HttpMethod, ModelBinding, ModelSelector, OperationKind, ProtocolQuirks, ProviderAuthHint,',
        '    ProviderClass, ProviderModelDescriptor, ProviderPluginDescriptor, TransportKind,',
        '    VerificationStatus, WireProtocol,',
        '};',
        '',
    ])
    return prelude + '\n\n'.join(blocks) + '\n'


def main() -> int:
    TARGET_DIR.mkdir(parents=True, exist_ok=True)
    TARGET_FILE.write_text(render_file(), encoding='utf-8')
    print(TARGET_FILE.relative_to(ROOT))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
