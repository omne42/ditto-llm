#!/usr/bin/env python3
from __future__ import annotations

import json
from collections import OrderedDict
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
SOURCE_DIR = ROOT / 'catalog' / 'provider_models'
TARGET_DIR = ROOT / 'catalog' / 'providers'
SCHEMA_VERSION = 'ditto.provider_catalog.v1'


Json = dict[str, Any]


def load_source(name: str) -> Json:
    path = SOURCE_DIR / f'{name}.json'
    with path.open('r', encoding='utf-8') as f:
        return json.load(f)


def ensure_dir(path: Path) -> None:
    path.mkdir(parents=True, exist_ok=True)


def compact_dict(value: Any) -> Any:
    if isinstance(value, dict):
        out = OrderedDict()
        for key, item in value.items():
            compacted = compact_dict(item)
            if compacted in (None, '', [], {}):
                continue
            out[key] = compacted
        return out
    if isinstance(value, list):
        out = []
        for item in value:
            compacted = compact_dict(item)
            if compacted in (None, '', [], {}):
                continue
            out.append(compacted)
        return out
    return value


def dedupe(values: list[str]) -> list[str]:
    out: list[str] = []
    for value in values:
        if value and value not in out:
            out.append(value)
    return out


def parse_openai_modalities(modalities: Json | None) -> Json:
    inputs: list[str] = []
    outputs: list[str] = []
    if not modalities:
        return {'input': inputs, 'output': outputs}
    for modality, direction in modalities.items():
        if direction in {'input_only', 'input_and_output'}:
            inputs.append(modality)
        if direction in {'output_only', 'input_and_output'}:
            outputs.append(modality)
    return {'input': inputs, 'output': outputs}


def parse_google_modalities(model: Json) -> Json:
    supported = model.get('supported_data_types') or {}
    return {
        'input': list(supported.get('input') or []),
        'output': list(supported.get('output') or []),
    }


def parse_anthropic_modalities(model: Json) -> Json:
    return {
        'input': list(model.get('input_modalities') or []),
        'output': list(model.get('output_modalities') or []),
    }


def infer_openai_family(model_id: str) -> str:
    if model_id.startswith('gpt-') or model_id.startswith('chatgpt-'):
        if 'image' in model_id:
            return 'image'
        if 'realtime' in model_id:
            return 'realtime'
        if 'transcribe' in model_id or 'audio' in model_id:
            return 'gpt-audio'
        return 'gpt'
    if model_id.startswith('o') and len(model_id) > 1 and model_id[1].isdigit():
        return 'o'
    if model_id.startswith('codex') or '-codex' in model_id:
        return 'codex'
    if model_id.startswith('dall-e'):
        return 'dall-e'
    if model_id.startswith('text-embedding'):
        return 'embedding'
    if 'moderation' in model_id:
        return 'moderation'
    if model_id.startswith('whisper'):
        return 'whisper'
    if model_id.startswith('tts-') or model_id.endswith('-tts'):
        return 'tts'
    if model_id.startswith('sora'):
        return 'sora'
    if model_id.startswith('computer-use'):
        return 'computer-use'
    return 'openai'


def infer_google_family(model_id: str) -> str:
    for prefix, family in (
        ('gemini-', 'gemini'),
        ('gemma-', 'gemma'),
        ('medgemma', 'medgemma'),
        ('signgemma', 'signgemma'),
        ('imagen-', 'imagen'),
        ('veo-', 'veo'),
        ('lyria', 'lyria'),
    ):
        if model_id.startswith(prefix):
            return family
    return 'google'


def infer_anthropic_family(model_id: str) -> str:
    return 'claude'


def infer_kimi_family(model_id: str) -> str:
    if model_id.startswith('kimi-k2.5'):
        return 'kimi-k2.5'
    if model_id.startswith('kimi-k2'):
        return 'kimi-k2'
    if model_id.startswith('moonshot-v1'):
        return 'moonshot-v1'
    return 'kimi'


def normalize_openai_stage(stage: str | None) -> tuple[str, str | None]:
    if stage == 'preview':
        return 'preview', 'preview'
    if stage == 'legacy':
        return 'legacy', 'legacy'
    if stage in {'recommended', 'default', 'latest'}:
        return 'active', stage
    return 'active', stage


def normalize_google_lifecycle(model_id: str, versions: list[Json]) -> tuple[str, str | None]:
    channels = [item.get('channel') for item in versions if item.get('channel')]
    if 'stable' in channels or 'stable_faster' in channels or 'stable_ultra' in channels:
        return 'active', 'stable'
    if 'preview' in channels or 'preview' in model_id:
        return 'preview', 'preview'
    if 'experimental' in channels or '-exp-' in model_id or model_id.endswith('-exp'):
        return 'experimental', 'experimental'
    return 'active', channels[0] if channels else None


OPENAI_INTERFACES: Json = OrderedDict(
    [
        ('response', OrderedDict([
            ('object', 'response'),
            ('display_name', 'Responses API'),
            ('protocol', 'openai_responses'),
            ('transport', 'http'),
            ('method', 'POST'),
            ('path', '/v1/responses'),
        ])),
        ('chat.completion', OrderedDict([
            ('object', 'chat.completion'),
            ('display_name', 'Chat Completions API'),
            ('protocol', 'openai_chat_completions'),
            ('transport', 'http'),
            ('method', 'POST'),
            ('path', '/v1/chat/completions'),
        ])),
        ('text.completion', OrderedDict([
            ('object', 'text.completion'),
            ('display_name', 'Legacy Text Completions API'),
            ('protocol', 'openai_text_completions'),
            ('transport', 'http'),
            ('method', 'POST'),
            ('path', '/v1/completions'),
        ])),
        ('embedding', OrderedDict([
            ('object', 'embedding'),
            ('display_name', 'Embeddings API'),
            ('protocol', 'openai_embeddings'),
            ('transport', 'http'),
            ('method', 'POST'),
            ('path', '/v1/embeddings'),
        ])),
        ('image.generation', OrderedDict([
            ('object', 'image.generation'),
            ('display_name', 'Image Generation API'),
            ('protocol', 'openai_images'),
            ('transport', 'http'),
            ('method', 'POST'),
            ('path', '/v1/images/generations'),
        ])),
        ('audio.transcription', OrderedDict([
            ('object', 'audio.transcription'),
            ('display_name', 'Audio Transcriptions API'),
            ('protocol', 'openai_audio_transcriptions'),
            ('transport', 'http'),
            ('method', 'POST'),
            ('path', '/v1/audio/transcriptions'),
        ])),
        ('audio.translation', OrderedDict([
            ('object', 'audio.translation'),
            ('display_name', 'Audio Translations API'),
            ('protocol', 'openai_audio_translations'),
            ('transport', 'http'),
            ('method', 'POST'),
            ('path', '/v1/audio/translations'),
        ])),
        ('audio.speech', OrderedDict([
            ('object', 'audio.speech'),
            ('display_name', 'Audio Speech API'),
            ('protocol', 'openai_audio_speech'),
            ('transport', 'http'),
            ('method', 'POST'),
            ('path', '/v1/audio/speech'),
        ])),
        ('realtime.session', OrderedDict([
            ('object', 'realtime.session'),
            ('display_name', 'Realtime API'),
            ('protocol', 'openai_realtime'),
            ('transport', 'websocket'),
            ('path', '/v1/realtime'),
        ])),
        ('moderation', OrderedDict([
            ('object', 'moderation'),
            ('display_name', 'Moderations API'),
            ('protocol', 'openai_moderations'),
            ('transport', 'http'),
            ('method', 'POST'),
            ('path', '/v1/moderations'),
        ])),
        ('video.generation', OrderedDict([
            ('object', 'video.generation'),
            ('display_name', 'Video Generation API'),
            ('protocol', 'openai_video_generation'),
            ('transport', 'http'),
            ('method', 'POST'),
            ('path', '/v1/videos'),
            ('notes', 'Path is inferred from current Sora API naming and should be treated as provisional.'),
        ])),
    ]
)


GOOGLE_INTERFACES: Json = OrderedDict(
    [
        ('content.generate', OrderedDict([
            ('object', 'content.generate'),
            ('display_name', 'Generate Content'),
            ('protocol', 'gemini_generate_content'),
            ('transport', 'http'),
            ('method', 'POST'),
            ('path_template', '/v1beta/models/{model}:generateContent'),
        ])),
        ('content.generate_stream', OrderedDict([
            ('object', 'content.generate_stream'),
            ('display_name', 'Stream Generate Content'),
            ('protocol', 'gemini_stream_generate_content'),
            ('transport', 'sse'),
            ('method', 'POST'),
            ('path_template', '/v1beta/models/{model}:streamGenerateContent?alt=sse'),
        ])),
        ('content.batch', OrderedDict([
            ('object', 'content.batch'),
            ('display_name', 'Batch Generate Content'),
            ('protocol', 'gemini_batch_generate_content'),
            ('transport', 'http'),
            ('method', 'POST'),
            ('path_template', '/v1beta/models/{model}:batchGenerateContent'),
        ])),
        ('embedding', OrderedDict([
            ('object', 'embedding'),
            ('display_name', 'Embed Content'),
            ('protocol', 'gemini_embed_content'),
            ('transport', 'http'),
            ('method', 'POST'),
            ('path_template', '/v1beta/models/{model}:embedContent'),
        ])),
        ('embedding.batch', OrderedDict([
            ('object', 'embedding.batch'),
            ('display_name', 'Batch Embed Content'),
            ('protocol', 'gemini_batch_embed_contents'),
            ('transport', 'http'),
            ('method', 'POST'),
            ('path_template', '/v1beta/models/{model}:batchEmbedContents'),
        ])),
        ('live.session', OrderedDict([
            ('object', 'live.session'),
            ('display_name', 'Live API Session'),
            ('protocol', 'gemini_live'),
            ('transport', 'websocket'),
            ('path', 'google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent'),
        ])),
        ('speech.generate', OrderedDict([
            ('object', 'speech.generate'),
            ('display_name', 'Text To Speech'),
            ('protocol', 'gemini_generate_content'),
            ('transport', 'http'),
            ('method', 'POST'),
            ('path_template', '/v1beta/models/{model}:generateContent'),
            ('notes', 'Semantic TTS interface carried over the Gemini generateContent endpoint.'),
        ])),
        ('image.generate', OrderedDict([
            ('object', 'image.generate'),
            ('display_name', 'Image Generation'),
            ('protocol', 'google_predict'),
            ('transport', 'http'),
            ('method', 'POST'),
            ('path_template', '/v1beta/models/{model}:predict'),
        ])),
        ('video.generate', OrderedDict([
            ('object', 'video.generate'),
            ('display_name', 'Video Generation'),
            ('protocol', 'google_predict_long_running'),
            ('transport', 'http'),
            ('method', 'POST'),
            ('path_template', '/v1beta/models/{model}:predictLongRunning'),
        ])),
        ('music.session', OrderedDict([
            ('object', 'music.session'),
            ('display_name', 'Realtime Music Session'),
            ('protocol', 'google_music_live'),
            ('transport', 'websocket'),
            ('path', 'google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateMusic'),
        ])),
    ]
)


ANTHROPIC_INTERFACES: Json = OrderedDict(
    [
        ('message', OrderedDict([
            ('object', 'message'),
            ('display_name', 'Messages API'),
            ('protocol', 'anthropic_messages'),
            ('transport', 'http'),
            ('method', 'POST'),
            ('path', '/v1/messages'),
        ])),
        ('message.count_tokens', OrderedDict([
            ('object', 'message.count_tokens'),
            ('display_name', 'Message Count Tokens API'),
            ('protocol', 'anthropic_messages_count_tokens'),
            ('transport', 'http'),
            ('method', 'POST'),
            ('path', '/v1/messages/count_tokens'),
        ])),
        ('legacy.complete', OrderedDict([
            ('object', 'legacy.complete'),
            ('display_name', 'Legacy Complete API'),
            ('protocol', 'anthropic_complete'),
            ('transport', 'http'),
            ('method', 'POST'),
            ('path', '/v1/complete'),
            ('notes', 'Retained for Claude 1.x/2.x style legacy completions.'),
        ])),
    ]
)


KIMI_INTERFACES: Json = OrderedDict(
    [
        ('chat.completion', OrderedDict([
            ('object', 'chat.completion'),
            ('display_name', 'Chat Completions API'),
            ('protocol', 'openai_chat_completions'),
            ('transport', 'http'),
            ('method', 'POST'),
            ('path', '/v1/chat/completions'),
        ])),
        ('model.list', OrderedDict([
            ('object', 'model.list'),
            ('display_name', 'List Models API'),
            ('protocol', 'openai_models_list'),
            ('transport', 'http'),
            ('method', 'GET'),
            ('path', '/v1/models'),
        ])),
    ]
)


MODALITY_CAPABILITY_KEYS = (
    'text',
    'image',
    'audio',
    'video',
    'pdf',
    'embeddings',
)


def modality_capabilities(modalities: Json) -> Json:
    inputs = set(modalities.get('input') or [])
    outputs = set(modalities.get('output') or [])
    if not inputs and not outputs:
        return OrderedDict()
    out = OrderedDict()
    for key in MODALITY_CAPABILITY_KEYS:
        out[f'{key}_input'] = key in inputs
        out[f'{key}_output'] = key in outputs
    out['multimodal_input'] = len(inputs - {'text'}) > 0 or len(inputs) > 1
    out['multimodal_output'] = len(outputs - {'text'}) > 0 or len(outputs) > 1
    return out


LEGACY_OPENAI_CHAT_ONLY = {
    'gpt-3.5-turbo',
    'gpt-4',
    'gpt-4-turbo',
    'gpt-4-turbo-preview',
    'chatgpt-4o-latest',
}
OPENAI_RESPONSE_ONLY_FAMILIES = {'o', 'codex', 'computer-use'}


def map_openai_interfaces(model_id: str, family: str) -> list[str]:
    if model_id.startswith('text-embedding'):
        return ['embedding']
    if 'moderation' in model_id:
        return ['moderation']
    if model_id == 'whisper-1':
        return ['audio.transcription', 'audio.translation']
    if 'transcribe' in model_id:
        return ['audio.transcription']
    if model_id.startswith('tts-') or model_id.endswith('-tts'):
        return ['audio.speech']
    if 'realtime' in model_id:
        return ['realtime.session']
    if model_id.startswith('dall-e') or model_id.startswith('gpt-image') or model_id == 'chatgpt-image-latest':
        return ['image.generation']
    if model_id.startswith('sora'):
        return ['video.generation']
    if model_id in {'babbage-002', 'davinci-002'}:
        return ['text.completion']
    if family == 'gpt-audio' or 'audio-preview' in model_id or 'search-preview' in model_id:
        return ['chat.completion']
    if model_id.endswith('-chat-latest') or model_id in LEGACY_OPENAI_CHAT_ONLY:
        return ['chat.completion']
    if family in OPENAI_RESPONSE_ONLY_FAMILIES:
        return ['response']
    return ['response', 'chat.completion']


MODERN_ANTHROPIC_PREFIXES = (
    'claude-3-',
    'claude-3.5-',
    'claude-3.7-',
    'claude-opus-4',
    'claude-sonnet-4',
    'claude-haiku-4',
)


def map_google_interfaces(model_id: str, model: Json) -> list[str]:
    outputs = set((model.get('supported_data_types') or {}).get('output') or [])
    capabilities = model.get('capabilities') or {}
    batch_supported = capabilities.get('batch_api') == 'supported'
    live_supported = capabilities.get('live_api') == 'supported'

    if model_id.startswith('gemini-embedding'):
        out = ['embedding']
        if batch_supported:
            out.append('embedding.batch')
        return out
    if model_id.startswith('imagen-'):
        return ['image.generate']
    if model_id.startswith('veo-'):
        return ['video.generate']
    if model_id.startswith('lyria'):
        return ['music.session']
    if live_supported or 'live' in model_id or 'native-audio' in model_id:
        return ['live.session']
    if model_id.endswith('-tts') or 'preview-tts' in model_id:
        return ['speech.generate']

    out = ['content.generate']
    if outputs == {'text'}:
        out.append('content.generate_stream')
    if batch_supported:
        out.append('content.batch')
    return out


def map_anthropic_interfaces(model_id: str) -> list[str]:
    if model_id.startswith(MODERN_ANTHROPIC_PREFIXES):
        return ['message', 'message.count_tokens']
    return ['legacy.complete']


OPENAI_CAPABILITY_KEYS = [
    'streaming',
    'function_calling',
    'structured_outputs',
    'fine_tuning',
    'distillation',
    'predicted_outputs',
    'web_search',
    'file_search',
    'image_generation',
    'code_interpreter',
    'hosted_shell',
    'apply_patch',
    'skills',
    'computer_use',
    'mcp',
    'tool_search',
]
GOOGLE_CAPABILITY_KEYS = [
    'batch_api',
    'cached_content',
    'code_execution',
    'function_calling',
    'google_search',
    'live_api',
    'search_grounding',
    'structured_outputs',
    'system_instructions',
    'thinking',
    'tuning',
    'url_context',
]
ANTHROPIC_CAPABILITY_KEYS = [
    'streaming',
    'tool_use',
    'vision',
    'prompt_caching',
    'extended_thinking',
    'pdf_support',
    'citations',
    'search_results',
]
KIMI_CAPABILITY_KEYS = [
    'streaming',
    'function_calling',
    'structured_outputs',
    'context_caching',
    'partial_mode',
    'web_search',
    'reasoning',
    'vision',
    'video_understanding',
]


def normalize_openai_capabilities(model: Json, interfaces: list[str], modalities: Json) -> Json:
    features = model.get('features') or {}
    tools = model.get('tools') or {}
    out = OrderedDict((key, None) for key in OPENAI_CAPABILITY_KEYS)
    for key, value in features.items():
        out[key] = value
    for key, value in tools.items():
        out[key] = value
    inferred_streaming = 'realtime.session' in interfaces or 'response' in interfaces or 'chat.completion' in interfaces
    out['streaming'] = bool(out.get('streaming') or inferred_streaming)
    out.update(modality_capabilities(modalities))
    return compact_dict(out)


FEATURE_NAME_MAP = {
    'tool_use': 'tool_use',
    'vision': 'vision',
    'prompt_caching': 'prompt_caching',
    'extended_thinking': 'extended_thinking',
    'pdf_support': 'pdf_support',
    'citations': 'citations',
    'search_results': 'search_results',
}


def normalize_google_capabilities(model: Json, interfaces: list[str], modalities: Json) -> Json:
    raw = model.get('capabilities') or {}
    out = OrderedDict()
    for key in GOOGLE_CAPABILITY_KEYS:
        if key in raw:
            out[key] = raw[key] == 'supported'
    out['streaming'] = 'content.generate_stream' in interfaces or 'live.session' in interfaces
    out.update(modality_capabilities(modalities))
    return compact_dict(out)


def normalize_anthropic_capabilities(model: Json, interfaces: list[str], modalities: Json) -> Json:
    raw = model.get('features') or {}
    out = OrderedDict()
    for source_key, target_key in FEATURE_NAME_MAP.items():
        if source_key in raw:
            out[target_key] = raw[source_key]
    out['streaming'] = 'message' in interfaces
    out['function_calling'] = raw.get('tool_use')
    out['structured_outputs'] = raw.get('tool_use')
    out.update(modality_capabilities(modalities))
    return compact_dict(out)


def normalize_kimi_capabilities(model: Json, interfaces: list[str], modalities: Json) -> Json:
    raw = model.get('features') or {}
    out = OrderedDict()
    for key in KIMI_CAPABILITY_KEYS:
        if key in raw:
            out[key] = raw[key]
    out['streaming'] = bool(out.get('streaming') or ('chat.completion' in interfaces))
    out.update(modality_capabilities(modalities))
    return compact_dict(out)


def normalize_openai_model(model_id: str, model: Json) -> Json:
    modalities = parse_openai_modalities(model.get('modalities'))
    family = infer_openai_family(model_id)
    interfaces = map_openai_interfaces(model_id, family)
    status, release_channel = normalize_openai_stage(model.get('stage'))
    source_urls = dedupe([model.get('source_url')])
    entry = OrderedDict([
        ('identity', OrderedDict([
            ('catalog_id', model_id),
            ('provider_model_id', model_id),
            ('brand', 'openai'),
            ('family', family),
            ('snapshot_ids', list((model.get('revisions') or {}).get('snapshots') or [])),
        ])),
        ('display_name', model.get('display_name')),
        ('tagline', model.get('tagline')),
        ('summary', model.get('summary')),
        ('release_channel', release_channel),
        ('lifecycle', OrderedDict([
            ('status', status),
            ('stage', model.get('stage')),
            ('knowledge_cutoff', model.get('knowledge_cutoff')),
        ])),
        ('modalities', modalities),
        ('limits', OrderedDict([
            ('context_window_tokens', model.get('context_window')),
            ('max_output_tokens', model.get('max_output_tokens')),
        ])),
        ('capabilities', normalize_openai_capabilities(model, interfaces, modalities)),
        ('primary_interface_id', interfaces[0]),
        ('supported_interface_ids', interfaces),
        ('source_urls', source_urls),
        ('vendor_metadata', OrderedDict([
            ('performance', model.get('performance')),
            ('speed', model.get('speed')),
            ('input_summary', model.get('input')),
            ('output_summary', model.get('output')),
            ('revisions', model.get('revisions')),
        ])),
    ])
    return compact_dict(entry)


def normalize_google_model(model_id: str, model: Json) -> Json:
    modalities = parse_google_modalities(model)
    family = infer_google_family(model_id)
    interfaces = map_google_interfaces(model_id, model)
    versions = list(model.get('versions') or [])
    status, release_channel = normalize_google_lifecycle(model_id, versions)
    limits = model.get('limits') or {}
    source_urls = dedupe([model.get('source_url')])
    entry = OrderedDict([
        ('identity', OrderedDict([
            ('catalog_id', model_id),
            ('provider_model_id', model.get('model_code') or model_id),
            ('brand', 'google'),
            ('family', family),
            ('version_ids', [item['model'] for item in versions if item.get('model')]),
        ])),
        ('display_name', model.get('display_name')),
        ('summary', model.get('summary')),
        ('release_channel', release_channel),
        ('lifecycle', OrderedDict([
            ('status', status),
            ('latest_update', model.get('latest_update')),
        ])),
        ('modalities', modalities),
        ('limits', OrderedDict([
            ('input_token_limit', limits.get('input_token_limit')),
            ('output_token_limit', limits.get('output_token_limit')),
            ('output_dimension_size', limits.get('output_dimension_size')),
            ('output_images', limits.get('output_images')),
        ])),
        ('capabilities', normalize_google_capabilities(model, interfaces, modalities)),
        ('primary_interface_id', interfaces[0]),
        ('supported_interface_ids', interfaces),
        ('source_urls', source_urls),
        ('vendor_metadata', OrderedDict([
            ('model_code', model.get('model_code')),
            ('versions', versions),
        ])),
    ])
    return compact_dict(entry)


def normalize_anthropic_model(model_id: str, model: Json) -> Json:
    modalities = parse_anthropic_modalities(model)
    interfaces = map_anthropic_interfaces(model_id)
    source_urls = dedupe([model.get('source_url'), model.get('lifecycle_source_url')])
    alternate_ids = OrderedDict([
        ('api_alias', model.get('api_alias')),
        ('bedrock_model_id', model.get('bedrock_model_id')),
        ('vertex_model_id', model.get('vertex_model_id')),
    ])
    entry = OrderedDict([
        ('identity', OrderedDict([
            ('catalog_id', model_id),
            ('provider_model_id', model.get('api_model_id') or model_id),
            ('brand', 'anthropic'),
            ('family', infer_anthropic_family(model_id)),
            ('alternate_ids', compact_dict(alternate_ids)),
        ])),
        ('display_name', model.get('display_name')),
        ('summary', model.get('description')),
        ('release_channel', 'stable' if model.get('status') == 'active' else None),
        ('lifecycle', OrderedDict([
            ('status', model.get('status')),
            ('deprecated_on', model.get('deprecated_on')),
            ('retirement_date', model.get('retirement_date')),
            ('not_retired_before', model.get('not_retired_before')),
            ('recommended_replacement', model.get('recommended_replacement')),
            ('training_data_cutoff', model.get('training_data_cutoff')),
            ('knowledge_cutoff', model.get('reliable_knowledge_cutoff')),
        ])),
        ('modalities', modalities),
        ('limits', OrderedDict([
            ('context_window_tokens', model.get('context_window_tokens')),
            ('beta_context_window_tokens', model.get('beta_context_window_tokens')),
            ('max_output_tokens', model.get('max_output_tokens')),
        ])),
        ('capabilities', normalize_anthropic_capabilities(model, interfaces, modalities)),
        ('primary_interface_id', interfaces[0]),
        ('supported_interface_ids', interfaces),
        ('source_urls', source_urls),
        ('vendor_metadata', OrderedDict([
            ('comparative_latency', model.get('comparative_latency')),
            ('pricing', model.get('pricing')),
            ('beta_headers', model.get('beta_headers')),
        ])),
    ])
    return compact_dict(entry)


def normalize_kimi_model(model_id: str, model: Json) -> Json:
    modalities = parse_openai_modalities(model.get('modalities'))
    stage = model.get('stage')
    interfaces = ['chat.completion']
    status = 'preview' if stage == 'preview' else 'active'
    source_urls = dedupe([
        model.get('source_url'),
        model.get('availability_source_url'),
        model.get('pricing_source_url'),
        model.get('release_notes_url'),
    ])
    entry = OrderedDict([
        ('identity', OrderedDict([
            ('catalog_id', model_id),
            ('provider_model_id', model_id),
            ('brand', 'kimi'),
            ('family', infer_kimi_family(model_id)),
        ])),
        ('display_name', model.get('display_name')),
        ('summary', model.get('summary')),
        ('release_channel', stage),
        ('lifecycle', OrderedDict([
            ('status', status),
            ('stage', stage),
        ])),
        ('modalities', modalities),
        ('limits', OrderedDict([
            ('context_window_tokens', model.get('context_window')),
        ])),
        ('capabilities', normalize_kimi_capabilities(model, interfaces, modalities)),
        ('primary_interface_id', interfaces[0]),
        ('supported_interface_ids', interfaces),
        ('source_urls', source_urls),
        ('vendor_metadata', OrderedDict([
            ('input_summary', model.get('input')),
            ('output_summary', model.get('output')),
            ('speed', model.get('speed')),
            ('pricing_source_url', model.get('pricing_source_url')),
            ('availability_source_url', model.get('availability_source_url')),
            ('release_notes_url', model.get('release_notes_url')),
        ])),
    ])
    return compact_dict(entry)


PROVIDERS = OrderedDict([
    ('openai', OrderedDict([
        ('interfaces', OPENAI_INTERFACES),
        ('normalize_model', normalize_openai_model),
    ])),
    ('kimi', OrderedDict([
        ('interfaces', KIMI_INTERFACES),
        ('normalize_model', normalize_kimi_model),
    ])),
    ('google', OrderedDict([
        ('interfaces', GOOGLE_INTERFACES),
        ('normalize_model', normalize_google_model),
    ])),
    ('anthropic', OrderedDict([
        ('interfaces', ANTHROPIC_INTERFACES),
        ('normalize_model', normalize_anthropic_model),
    ])),
])


def normalize_provider(name: str) -> Json:
    source = load_source(name)
    provider = source['provider']
    interfaces = PROVIDERS[name]['interfaces']
    normalize_model = PROVIDERS[name]['normalize_model']
    models = OrderedDict()
    for model_id, model in source['models'].items():
        normalized = normalize_model(model_id, model)
        if not normalized.get('primary_interface_id'):
            raise ValueError(f'{name}:{model_id} missing primary_interface_id')
        models[model_id] = normalized
    return compact_dict(OrderedDict([
        ('schema_version', SCHEMA_VERSION),
        ('normalized_from', f'catalog/provider_models/{name}.json'),
        ('provider', provider),
        ('interfaces', interfaces),
        ('models', models),
    ]))


def write_catalog(name: str) -> Path:
    ensure_dir(TARGET_DIR)
    output = TARGET_DIR / f'{name}.json'
    data = normalize_provider(name)
    output.write_text(json.dumps(data, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
    return output


def main() -> int:
    written = [write_catalog(name) for name in PROVIDERS]
    for path in written:
        print(path.relative_to(ROOT))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
