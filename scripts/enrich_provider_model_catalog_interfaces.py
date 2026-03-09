#!/usr/bin/env python3
from __future__ import annotations

import re
from collections import OrderedDict
from pathlib import Path

try:
    import tomllib as toml
except ModuleNotFoundError:
    import tomli as toml

from provider_model_catalog_json import write_json_sidecar

CATALOG_DIR = Path(__file__).resolve().parents[1] / 'catalog' / 'provider_models'
TARGET_FILES = [
    'openai.toml',
    'google.toml',
    'anthropic.toml',
    'kimi.toml',
    'openrouter.toml',
    'bailian.toml',
    'minimax.toml',
    'qianfan.toml',
]

OPENAI_RESPONSES_URL = 'https://api.openai.com/v1/responses'
OPENAI_CHAT_URL = 'https://api.openai.com/v1/chat/completions'
OPENAI_COMPLETIONS_URL = 'https://api.openai.com/v1/completions'
OPENAI_EMBEDDINGS_URL = 'https://api.openai.com/v1/embeddings'
OPENAI_MODERATIONS_URL = 'https://api.openai.com/v1/moderations'
OPENAI_AUDIO_SPEECH_URL = 'https://api.openai.com/v1/audio/speech'
OPENAI_AUDIO_TRANSCRIPTIONS_URL = 'https://api.openai.com/v1/audio/transcriptions'
OPENAI_REALTIME_URL = 'wss://api.openai.com/v1/realtime'
OPENAI_IMAGE_GEN_URL = 'https://api.openai.com/v1/images/generations'
OPENAI_IMAGE_EDIT_URL = 'https://api.openai.com/v1/images/edits'
OPENAI_VIDEO_GEN_URL = 'https://api.openai.com/v1/videos'

GOOGLE_GENERATE_CONTENT_URL = 'https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent'
GOOGLE_EMBED_CONTENT_URL = 'https://generativelanguage.googleapis.com/v1beta/models/{model}:embedContent'
GOOGLE_PREDICT_URL = 'https://generativelanguage.googleapis.com/v1beta/models/{model}:predict'
GOOGLE_PREDICT_LONG_RUNNING_URL = 'https://generativelanguage.googleapis.com/v1beta/models/{model}:predictLongRunning'
GOOGLE_LIVE_WS_URL = 'wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent'

ANTHROPIC_MESSAGES_URL = 'https://api.anthropic.com/v1/messages'
KIMI_CHAT_URL = 'https://api.moonshot.cn/v1/chat/completions'
OPENROUTER_CHAT_URL = 'https://openrouter.ai/api/v1/chat/completions'

MINIMAX_CHAT_URL = 'https://api.minimaxi.com/v1/text/chatcompletion_v2'
MINIMAX_ANTHROPIC_URL = 'https://api.minimaxi.com/anthropic/v1/messages'

QIANFAN_CHAT_URL = 'https://qianfan.baidubce.com/v2/chat/completions'
QIANFAN_EMBEDDINGS_URL = 'https://qianfan.baidubce.com/v2/embeddings'
QIANFAN_RERANK_URL = 'https://qianfan.baidubce.com/v2/rerank'
QIANFAN_IMAGE_GEN_URL = 'https://qianfan.baidubce.com/v2/images/generations'
QIANFAN_IMAGE_EDIT_URL = 'https://qianfan.baidubce.com/v2/images/edits'
QIANFAN_VIDEO_GEN_URL = 'https://qianfan.baidubce.com/beta/video/generations/qianfan-video'

BAILIAN_CHAT_URL = 'https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions'
BAILIAN_TEXT_EMBEDDING_URL = 'https://dashscope.aliyuncs.com/api/v1/services/embeddings/text-embedding/text-embedding'
BAILIAN_MULTIMODAL_EMBEDDING_URL = 'https://dashscope.aliyuncs.com/api/v1/services/embeddings/multimodal-embedding/multimodal-embedding'
BAILIAN_QWEN_MULTIMODAL_URL = 'https://dashscope.aliyuncs.com/api/v1/services/aigc/multimodal-generation/generation'
BAILIAN_IMAGE_GENERATION_URL = 'https://dashscope.aliyuncs.com/api/v1/services/aigc/image-generation/generation'
BAILIAN_IMAGE2IMAGE_URL = 'https://dashscope.aliyuncs.com/api/v1/services/aigc/image2image/image-synthesis'
BAILIAN_VIDEO_GENERATION_URL = 'https://dashscope.aliyuncs.com/api/v1/services/aigc/video-generation/video-synthesis'
BAILIAN_IMAGE2VIDEO_URL = 'https://dashscope.aliyuncs.com/api/v1/services/aigc/image2video/video-synthesis'
BAILIAN_AUDIO_ASR_TRANSCRIPTION_URL = 'https://dashscope.aliyuncs.com/api/v1/services/audio/asr/transcription'
BAILIAN_AUDIO_TTS_CUSTOMIZATION_URL = 'https://dashscope.aliyuncs.com/api/v1/services/audio/tts/customization'
BAILIAN_RERANK_URL = 'https://dashscope.aliyuncs.com/api/v1/services/rerank/text-rerank/text-rerank'
BAILIAN_NLU_UNDERSTANDING_URL = 'https://dashscope.aliyuncs.com/api/v1/services/nlp/nlu/understanding'
BAILIAN_WS_INFERENCE_URL = 'wss://dashscope.aliyuncs.com/api-ws/v1/inference'
BAILIAN_WS_REALTIME_URL = 'wss://dashscope.aliyuncs.com/api-ws/v1/realtime'

OPENAI_RESPONSES_ONLY_SNIPPETS = (
    'responses api only',
    'only usable in the responses api',
    'only available in the responses api',
    "it's available in the responses api only",
    "it's only available in the responses api",
)
OPENAI_CHAT_ONLY_MODEL_IDS = {
    'gpt-3.5-turbo',
    'gpt-4',
    'gpt-4-turbo',
    'gpt-4-turbo-preview',
    'gpt-4o-search-preview',
    'gpt-4o-mini-search-preview',
}
OPENAI_REASONING_MODEL_IDS = {'codex-mini-latest'}
OPENAI_REASONING_MODEL_PREFIXES = ('gpt-5', 'gpt-oss-', 'o1', 'o3', 'o4')
OPENAI_PROMPT_CACHE_MODEL_IDS = {'codex-mini-latest', 'computer-use-preview'}
OPENAI_PROMPT_CACHE_MODEL_PREFIXES = (
    'chatgpt-4o',
    'gpt-4o',
    'gpt-4.1',
    'gpt-4.5',
    'gpt-5',
    'gpt-oss-',
    'o1',
    'o3',
    'o4',
)
OPENAI_TEXT_BEHAVIOR_SURFACES = {'chat.completion', 'responses', 'completion.legacy'}
OPENAI_SURFACE_TO_BEHAVIOR_OPERATION = {
    'chat.completion': 'chat.completion',
    'responses': 'response',
    'completion.legacy': 'text.completion',
}


QUOTED_STRING_RE = re.compile(r'^([A-Za-z0-9_.-]+)\s*=\s*"(.*)"\s*$')
RECORD_HEADER_RE = re.compile(r'^\[\[models\.(".+")\.records\]\]$')
MODEL_HEADER_RE = re.compile(r'^\[models\.(".+")\]$')


def toml_quote(value: str) -> str:
    escaped = (
        value.replace('\\', '\\\\')
        .replace('"', '\\"')
        .replace('\n', '\\n')
        .replace('\r', '\\r')
        .replace('\t', '\\t')
    )
    return f'"{escaped}"'


def toml_array(values: list[str]) -> str:
    return '[' + ', '.join(toml_quote(value) for value in values) + ']'


def normalize_text_list(model: dict) -> tuple[list[str], list[str]]:
    if 'modalities' in model:
        modalities = model['modalities']
        inputs: list[str] = []
        outputs: list[str] = []
        for modality, support in modalities.items():
            if support in {'input_only', 'input_and_output'}:
                inputs.append(modality)
            if support in {'output_only', 'input_and_output'}:
                outputs.append(modality)
        return inputs, outputs
    if 'supported_data_types' in model:
        info = model['supported_data_types']
        return list(info.get('input') or []), list(info.get('output') or [])
    return list(model.get('input_modalities') or []), list(model.get('output_modalities') or [])


def model_summary(model: dict) -> str:
    return ' '.join(
        str(model.get(key) or '')
        for key in ('summary', 'description', 'tagline', 'display_name')
    ).lower()


def joined_text(value: object) -> str:
    if isinstance(value, list):
        return ' / '.join(str(item) for item in value)
    return str(value or '')


def model_code_or_id(model_id: str, model: dict) -> str:
    return str(model.get('model_code') or model_id)


def openai_is_responses_only(model_id: str, model: dict) -> bool:
    if model_id == 'computer-use-preview':
        return True
    summary = model_summary(model)
    return any(snippet in summary for snippet in OPENAI_RESPONSES_ONLY_SNIPPETS)


def openai_is_chat_only(model_id: str, model: dict) -> bool:
    if model_id in OPENAI_CHAT_ONLY_MODEL_IDS:
        return True
    summary = model_summary(model)
    return (
        'usable in chat completions' in summary
        or 'with the chat completions api' in summary
        or 'using the chat completions api' in summary
    )


def openai_supports_reasoning(model_id: str, model: dict) -> bool:
    if model_id in OPENAI_REASONING_MODEL_IDS or model_id.startswith(OPENAI_REASONING_MODEL_PREFIXES):
        return True
    summary = model_summary(model)
    return any(
        snippet in summary
        for snippet in (
            'reasoning.effort supports',
            'configurable reasoning',
            'reasoning model',
            'think before they answer',
            'full chain-of-thought',
        )
    )


def openai_supports_prompt_caching(model_id: str) -> bool:
    return model_id in OPENAI_PROMPT_CACHE_MODEL_IDS or model_id.startswith(OPENAI_PROMPT_CACHE_MODEL_PREFIXES)


def infer_openai_surfaces(model_id: str, model: dict) -> list[str]:
    inputs, outputs = normalize_text_list(model)
    outputs_set = set(outputs)
    inputs_set = set(inputs)
    if model_id in {'babbage-002', 'davinci-002'}:
        return ['completion.legacy']
    if model_id.startswith('text-embedding-'):
        return ['embedding']
    if 'moderation' in model_id:
        return ['moderation']
    if model_id in {'tts-1', 'tts-1-hd', 'gpt-4o-mini-tts'}:
        return ['audio.speech']
    if model_id in {'whisper-1', 'gpt-4o-mini-transcribe', 'gpt-4o-transcribe', 'gpt-4o-transcribe-diarize'}:
        return ['audio.transcription']
    if 'realtime' in model_id or model_id in {'gpt-realtime', 'gpt-realtime-1.5', 'gpt-realtime-mini'}:
        return ['realtime.websocket']
    if model_id.startswith('sora-') or 'video' in outputs_set:
        return ['video.generation']
    if model_id.startswith('dall-e') or model_id.startswith('gpt-image') or model_id == 'chatgpt-image-latest':
        surfaces = ['image.generation']
        if 'image' in inputs_set:
            surfaces.append('image.edit')
        return surfaces
    if openai_is_responses_only(model_id, model):
        return ['responses']
    if openai_is_chat_only(model_id, model):
        return ['chat.completion']
    return ['chat.completion', 'responses']


def infer_google_surfaces(model_id: str, model: dict) -> list[str]:
    lowered = model_id.lower()
    _, outputs = normalize_text_list(model)
    outputs_set = set(outputs)
    if 'embedding' in lowered or 'embeddings' in outputs_set:
        return ['embedding']
    if 'live' in lowered or 'native-audio' in lowered or lowered == 'lyria':
        return ['realtime.websocket']
    if lowered.startswith('imagen'):
        return ['image.generation']
    if lowered.startswith('veo'):
        return ['video.generation']
    return ['generate.content']


def infer_anthropic_surfaces(model_id: str, model: dict) -> list[str]:
    return ['anthropic.messages']


def infer_kimi_surfaces(model_id: str, model: dict) -> list[str]:
    return ['chat.completion']


def infer_qianfan_surfaces(model_id: str, model: dict) -> list[str]:
    surfaces = list(model.get('api_surfaces') or [])
    if surfaces:
        return surfaces
    categories = ' '.join(model.get('categories') or [])
    lowered = model_id.lower()
    if '视频生成' in categories or 'video' in lowered:
        return ['video.generation']
    if 'ocr' in categories or 'ocr' in lowered:
        return ['ocr']
    if '多模态向量' in categories:
        return ['embedding.multimodal']
    if '文本向量' in categories or 'embedding' in lowered:
        return ['embedding']
    if '重排序' in categories or 'rerank' in lowered:
        return ['rerank']
    if '图像编辑' in categories:
        return ['image.edit']
    if '图像生成' in categories:
        return ['image.generation']
    return ['chat.completion']


def infer_surfaces(provider_id: str, model_id: str, model: dict) -> list[str]:
    return {
        'openai': infer_openai_surfaces,
        'google': infer_google_surfaces,
        'anthropic': infer_anthropic_surfaces,
        'kimi': infer_kimi_surfaces,
        'qianfan': infer_qianfan_surfaces,
    }[provider_id](model_id, model)


def bailian_is_multimodal_embedding(model_id: str) -> bool:
    lowered = model_id.lower()
    return lowered == 'multimodal-embedding-v1' or 'vl-embedding' in lowered or 'embedding-vision' in lowered


def bailian_endpoint_for_surface(surface: str, model_id: str, model: dict) -> str | None:
    lowered = model_id.lower()
    series = joined_text(model.get('series'))
    if surface == 'chat.completion':
        return BAILIAN_CHAT_URL
    if surface == 'embedding':
        return BAILIAN_MULTIMODAL_EMBEDDING_URL if bailian_is_multimodal_embedding(model_id) else BAILIAN_TEXT_EMBEDDING_URL
    if surface == 'rerank':
        return BAILIAN_RERANK_URL
    if surface == 'classification_or_extraction':
        return BAILIAN_NLU_UNDERSTANDING_URL
    if surface == 'image.translation':
        return BAILIAN_IMAGE2IMAGE_URL
    if surface == 'image.generation':
        if lowered.startswith('qwen-image') or '千问文生图' in series or '千问图像编辑' in series:
            return BAILIAN_QWEN_MULTIMODAL_URL
        return BAILIAN_IMAGE_GENERATION_URL
    if surface == 'image.edit':
        if lowered.startswith('qwen-image') or '千问图像编辑' in series:
            return BAILIAN_QWEN_MULTIMODAL_URL
        return BAILIAN_IMAGE_GENERATION_URL
    if surface == 'video.generation':
        if '基于首尾帧' in series:
            return BAILIAN_IMAGE2VIDEO_URL
        return BAILIAN_VIDEO_GENERATION_URL
    if surface == 'audio.speech':
        if lowered in {'qwen-voice-design', 'qwen-voice-enrollment'}:
            return BAILIAN_AUDIO_TTS_CUSTOMIZATION_URL
        if lowered.startswith('qwen') and 'realtime' in lowered:
            return BAILIAN_WS_REALTIME_URL
        if lowered.startswith('qwen'):
            return BAILIAN_QWEN_MULTIMODAL_URL
        if lowered.startswith('cosyvoice') or lowered.startswith('sambert') or '实时' in series:
            return BAILIAN_WS_INFERENCE_URL
        return BAILIAN_QWEN_MULTIMODAL_URL
    if surface == 'audio.transcription.realtime':
        return BAILIAN_WS_REALTIME_URL if lowered.startswith('qwen') else BAILIAN_WS_INFERENCE_URL
    if surface == 'audio.transcription':
        if lowered.startswith('qwen3-asr-flash-filetrans') or lowered.startswith('qwen3-livetranslate-flash'):
            return BAILIAN_AUDIO_ASR_TRANSCRIPTION_URL if 'realtime' not in lowered else BAILIAN_WS_REALTIME_URL
        if lowered.startswith('qwen3-asr-flash'):
            if 'realtime' in lowered:
                return BAILIAN_WS_REALTIME_URL
            return BAILIAN_CHAT_URL
        if lowered.startswith('qwen-audio-asr'):
            return BAILIAN_QWEN_MULTIMODAL_URL
        if lowered.startswith('qwen') and 'realtime' in lowered:
            return BAILIAN_WS_REALTIME_URL
        if 'realtime' in lowered:
            return BAILIAN_WS_INFERENCE_URL
        return BAILIAN_AUDIO_ASR_TRANSCRIPTION_URL
    return None


def endpoint_for_surface(provider_id: str, surface: str, model_id: str, model: dict) -> str | None:
    if provider_id == 'openai':
        return {
            'responses': OPENAI_RESPONSES_URL,
            'chat.completion': OPENAI_CHAT_URL,
            'completion.legacy': OPENAI_COMPLETIONS_URL,
            'embedding': OPENAI_EMBEDDINGS_URL,
            'moderation': OPENAI_MODERATIONS_URL,
            'audio.speech': OPENAI_AUDIO_SPEECH_URL,
            'audio.transcription': OPENAI_AUDIO_TRANSCRIPTIONS_URL,
            'realtime.websocket': OPENAI_REALTIME_URL,
            'image.generation': OPENAI_IMAGE_GEN_URL,
            'image.edit': OPENAI_IMAGE_EDIT_URL,
            'video.generation': OPENAI_VIDEO_GEN_URL,
        }.get(surface)
    if provider_id == 'google':
        return {
            'generate.content': GOOGLE_GENERATE_CONTENT_URL,
            'embedding': GOOGLE_EMBED_CONTENT_URL,
            'realtime.websocket': GOOGLE_LIVE_WS_URL,
            'image.generation': GOOGLE_PREDICT_URL,
            'video.generation': GOOGLE_PREDICT_LONG_RUNNING_URL,
        }.get(surface)
    if provider_id == 'anthropic':
        return ANTHROPIC_MESSAGES_URL if surface == 'anthropic.messages' else None
    if provider_id == 'kimi':
        return KIMI_CHAT_URL if surface == 'chat.completion' else None
    if provider_id == 'openrouter':
        return OPENROUTER_CHAT_URL if surface == 'chat.completion' else None
    if provider_id == 'minimax':
        return {
            'chat.completion': MINIMAX_CHAT_URL,
            'anthropic.messages': MINIMAX_ANTHROPIC_URL,
        }.get(surface)
    if provider_id == 'qianfan':
        return {
            'chat.completion': QIANFAN_CHAT_URL,
            'embedding': QIANFAN_EMBEDDINGS_URL,
            'embedding.multimodal': QIANFAN_EMBEDDINGS_URL,
            'rerank': QIANFAN_RERANK_URL,
            'image.generation': QIANFAN_IMAGE_GEN_URL,
            'image.edit': QIANFAN_IMAGE_EDIT_URL,
            'ocr': QIANFAN_CHAT_URL,
            'video.generation': QIANFAN_VIDEO_GEN_URL,
        }.get(surface)
    if provider_id == 'bailian':
        return bailian_endpoint_for_surface(surface, model_id, model)
    return None


def official_source_url_for(provider_id: str, surface: str, model_id: str, model: dict) -> str | None:
    lowered = model_id.lower()
    series = joined_text(model.get('series'))
    if provider_id == 'google':
        return {
            'generate.content': str(model.get('source_url') or ''),
            'embedding': str(model.get('source_url') or ''),
            'realtime.websocket': 'https://ai.google.dev/gemini-api/docs/live',
            'image.generation': 'https://ai.google.dev/api/imagen-api',
            'video.generation': 'https://ai.google.dev/gemini-api/docs/video',
        }.get(surface)
    if provider_id == 'minimax':
        return {
            'chat.completion': 'https://platform.minimaxi.com/docs/api-reference/text-openai-api',
            'anthropic.messages': 'https://platform.minimaxi.com/docs/api-reference/text-anthropic-api',
        }.get(surface)
    if provider_id == 'qianfan':
        return {
            'chat.completion': 'https://cloud.baidu.com/doc/qianfan-api/s/Wm95lyynv',
            'embedding': 'https://cloud.baidu.com/doc/qianfan-api/s/Fm7u3ropn',
            'embedding.multimodal': 'https://cloud.baidu.com/doc/qianfan-api/s/Fm7u3ropn',
            'rerank': 'https://cloud.baidu.com/doc/qianfan-api/s/2m7u4zt74',
            'image.generation': 'https://cloud.baidu.com/doc/qianfan-api/s/bm8wv3h6f',
            'image.edit': 'https://cloud.baidu.com/doc/qianfan-docs/s/Um8r1tpwy',
            'ocr': 'https://cloud.baidu.com/doc/qianfan-docs/s/Um8r1tpwy',
            'video.generation': 'https://cloud.baidu.com/doc/WENXINWORKSHOP/s/jm0n1b32t',
        }.get(surface)
    if provider_id != 'bailian':
        return None
    if surface == 'chat.completion':
        return 'https://help.aliyun.com/zh/model-studio/qwen-api-reference/'
    if surface == 'embedding':
        return 'https://help.aliyun.com/zh/model-studio/multimodal-embedding-api-reference' if bailian_is_multimodal_embedding(model_id) else 'https://help.aliyun.com/zh/model-studio/text-embedding-synchronous-api'
    if surface == 'rerank':
        return 'https://help.aliyun.com/zh/model-studio/text-rerank-api-reference'
    if surface == 'classification_or_extraction':
        return 'https://help.aliyun.com/zh/model-studio/opennlu-api'
    if surface == 'image.translation':
        return 'https://help.aliyun.com/zh/model-studio/qwen-mt-image-api'
    if surface == 'image.generation':
        if lowered.startswith('qwen-image') or '千问文生图' in series or '千问图像编辑' in series:
            return 'https://help.aliyun.com/zh/model-studio/qwen-image-api'
        return 'https://help.aliyun.com/zh/model-studio/wan-image-generation-api-reference'
    if surface == 'image.edit':
        if lowered.startswith('qwen-image') or '千问图像编辑' in series:
            return 'https://help.aliyun.com/zh/model-studio/qwen-image-edit-api'
        return 'https://help.aliyun.com/zh/model-studio/wan-image-generation-api-reference'
    if surface == 'video.generation':
        if '基于首尾帧' in series:
            return 'https://help.aliyun.com/zh/model-studio/image-to-video-by-first-and-last-frame-api-reference'
        if '图生视频-基于首帧' in series:
            return 'https://help.aliyun.com/zh/model-studio/image-to-video-api-reference/'
        return 'https://help.aliyun.com/zh/model-studio/text-to-video-api-reference'
    if surface == 'audio.speech':
        if lowered == 'qwen-voice-design':
            return 'https://help.aliyun.com/zh/model-studio/qwen-tts-voice-design'
        if lowered == 'qwen-voice-enrollment':
            return 'https://help.aliyun.com/zh/model-studio/qwen-tts-realtime'
        if lowered.startswith('qwen') and 'realtime' in lowered:
            return 'https://help.aliyun.com/zh/model-studio/qwen-tts-realtime'
        if lowered.startswith('qwen'):
            return 'https://help.aliyun.com/zh/model-studio/qwen-tts-api'
        if lowered.startswith('cosyvoice'):
            return 'https://help.aliyun.com/zh/model-studio/cosyvoice-large-model-for-speech-synthesis/'
        if lowered.startswith('sambert'):
            return 'https://help.aliyun.com/zh/model-studio/sambert-speech-synthesis/'
        return 'https://help.aliyun.com/zh/model-studio/speech-synthesis-api-reference/'
    if surface == 'audio.transcription.realtime':
        return 'https://help.aliyun.com/zh/model-studio/qwen-asr-realtime-api/' if lowered.startswith('qwen') else 'https://help.aliyun.com/zh/model-studio/speech-recognition-api-reference/'
    if surface == 'audio.transcription':
        if lowered.startswith('qwen'):
            return 'https://help.aliyun.com/zh/model-studio/qwen-asr-api-reference'
        if lowered.startswith('fun-asr') and 'realtime' in lowered:
            return 'https://help.aliyun.com/zh/model-studio/fun-asr-realtime-websocket-api'
        if lowered.startswith('paraformer') and 'realtime' in lowered:
            return 'https://help.aliyun.com/zh/model-studio/websocket-for-paraformer-real-time-service'
        return 'https://help.aliyun.com/zh/model-studio/speech-recognition-api-reference/'
    return None


def source_page_for(provider_id: str, surface: str, model: dict) -> str:
    mapping = {
        'openai': {
            'responses': 'responses_api',
            'chat.completion': 'chat_completions_api',
            'completion.legacy': 'completions_api',
            'embedding': 'embeddings_api',
            'moderation': 'moderations_api',
            'audio.speech': 'audio_speech_api',
            'audio.transcription': 'audio_transcriptions_api',
            'realtime.websocket': 'realtime_api',
            'image.generation': 'images_api',
            'image.edit': 'images_api',
            'video.generation': 'sora_api',
        },
        'google': {
            'generate.content': 'generate_content',
            'embedding': 'embed_content',
            'realtime.websocket': 'live_api',
            'image.generation': 'imagen_api',
            'video.generation': 'video_generation',
        },
        'anthropic': {'anthropic.messages': 'messages'},
        'kimi': {'chat.completion': 'chat_completion'},
        'openrouter': {'chat.completion': 'chat_completion'},
        'minimax': {
            'chat.completion': 'text_openai_api',
            'anthropic.messages': 'text_anthropic_api',
        },
        'qianfan': {
            'chat.completion': 'chat_completions',
            'embedding': 'embeddings',
            'rerank': 'rerank',
            'image.generation': 'image_generation',
            'image.edit': 'image_edit',
            'video.generation': 'video_generation',
            'embedding.multimodal': 'multimodal_embedding',
            'ocr': 'ocr',
        },
        'bailian': {
            'chat.completion': 'chat_completions',
            'embedding': 'embeddings',
            'image.generation': 'image_generation',
            'image.edit': 'image_edit',
            'image.translation': 'image_translation',
            'video.generation': 'video_generation',
            'audio.speech': 'audio_speech',
            'audio.transcription': 'audio_transcription',
            'audio.transcription.realtime': 'audio_transcription_realtime',
            'rerank': 'rerank',
            'classification_or_extraction': 'classification_or_extraction',
        },
    }
    return mapping.get(provider_id, {}).get(surface, 'model_list')


def section_for(provider_id: str, surface: str, model: dict) -> str:
    categories = ' / '.join(model.get('categories') or [])
    if categories:
        return categories
    mapping = {
        'responses': 'Responses API',
        'chat.completion': 'Chat Completions',
        'completion.legacy': 'Legacy Completions',
        'embedding': 'Embeddings',
        'moderation': 'Moderations',
        'audio.speech': 'Text to Speech',
        'audio.transcription': 'Transcription',
        'audio.transcription.realtime': 'Realtime Transcription',
        'realtime.websocket': 'Realtime / Live',
        'image.generation': 'Image Generation',
        'image.edit': 'Image Editing',
        'video.generation': 'Video Generation',
        'generate.content': 'Generate Content',
        'anthropic.messages': 'Messages API',
        'embedding.multimodal': 'Multimodal Embeddings',
        'ocr': 'OCR',
        'classification_or_extraction': 'Classification / Extraction',
    }
    return mapping.get(surface, 'Model API')


def notes_for(provider_id: str, surface: str, model_id: str, model: dict, inferred: bool) -> str:
    if provider_id == 'qianfan' and surface == 'video.generation':
        return 'Mapped from the current official Qianfan video generation docs, which expose the beta qianfan-video task creation endpoint.'
    if provider_id == 'qianfan' and inferred:
        return 'Endpoint surface inferred from the official Qianfan category or retirement listing because the source record did not preserve an explicit API reference.'
    if provider_id == 'bailian':
        return 'Endpoint mapped from the official Bailian API references for the corresponding model family and transport.'
    if provider_id == 'openrouter':
        return 'OpenRouter exposes currently listed LLM models through the OpenAI-compatible chat completions interface.'
    if provider_id == 'google' and surface == 'realtime.websocket':
        return 'Google Live API models use bidirectional websocket sessions rather than plain generateContent HTTP requests.'
    if provider_id == 'google' and surface in {'image.generation', 'video.generation'}:
        return 'Endpoint mapped from the official Google media generation docs that use predict or predictLongRunning operations.'
    if provider_id == 'minimax' and surface in {'chat.completion', 'anthropic.messages'}:
        return 'Mapped from the official MiniMax OpenAI-compatible or Anthropic-compatible API references.'
    if provider_id == 'openai' and surface == 'responses':
        return 'This model family is documented for the OpenAI Responses API or explicitly marked as Responses-only in the model docs.'
    if provider_id == 'openai' and surface == 'chat.completion':
        return 'This model family is documented for OpenAI chat completions-compatible request/response flows.'
    return 'API surface supplemented from the provider protocol and model capability metadata already recorded in this catalog.'


def format_endpoint(endpoint: str | None, model_id: str, model: dict) -> str | None:
    if endpoint is None:
        return None
    return endpoint.replace('{model}', model_code_or_id(model_id, model))


def build_records(provider_id: str, model_id: str, model: dict, surfaces: list[str], inferred_surfaces: bool) -> list[OrderedDict[str, str]]:
    records: list[OrderedDict[str, str]] = []
    fallback_source_url = str(
        model.get('availability_source_url')
        or model.get('source_url')
        or ''
    )
    for surface in surfaces:
        record: OrderedDict[str, str] = OrderedDict()
        record['table_kind'] = 'api_reference'
        record['source_url'] = official_source_url_for(provider_id, surface, model_id, model) or fallback_source_url
        record['source_page'] = source_page_for(provider_id, surface, model)
        record['section'] = section_for(provider_id, surface, model)
        record['api_surface'] = surface
        endpoint = format_endpoint(endpoint_for_surface(provider_id, surface, model_id, model), model_id, model)
        if endpoint is not None:
            record['endpoint'] = endpoint
        record['notes'] = notes_for(provider_id, surface, model_id, model, inferred_surfaces)
        records.append(record)
    return records


def render_toml_value(value: object) -> str:
    if isinstance(value, bool):
        return str(value).lower()
    if isinstance(value, list):
        return toml_array([str(item) for item in value])
    return toml_quote(str(value))


def render_record(model_id: str, record: OrderedDict[str, object]) -> list[str]:
    lines = [f'[[models.{toml_quote(model_id)}.records]]']
    for key, value in record.items():
        lines.append(f'{key} = {render_toml_value(value)}')
    return lines


def build_openai_behavior_records(model_id: str, model: dict, surfaces: list[str]) -> list[OrderedDict[str, object]]:
    if not any(surface in OPENAI_TEXT_BEHAVIOR_SURFACES for surface in surfaces):
        return []

    features = model.get('features') or {}
    reasoning_supported = openai_supports_reasoning(model_id, model)
    prompt_cache_supported = openai_supports_prompt_caching(model_id)
    tool_support = 'supported' if features.get('function_calling') else 'unsupported'
    response_only = openai_is_responses_only(model_id, model)
    chat_only = openai_is_chat_only(model_id, model)

    records: list[OrderedDict[str, object]] = []
    for surface in surfaces:
        if surface not in OPENAI_TEXT_BEHAVIOR_SURFACES:
            continue

        record: OrderedDict[str, object] = OrderedDict()
        record['table_kind'] = 'behavior'
        record['source_url'] = str(model.get('source_url') or '')
        record['source_page'] = 'model_behaviors'
        record['section'] = 'Runtime Semantics'
        record['operation'] = OPENAI_SURFACE_TO_BEHAVIOR_OPERATION[surface]

        if surface == 'completion.legacy':
            record['tool_calls'] = 'unsupported'
            record['tool_choice_required'] = 'unsupported'
            record['assistant_tool_followup'] = 'none'
            record['reasoning_output'] = 'unsupported'
            record['reasoning_activation'] = 'unavailable'
            record['context_cache_modes'] = []
            record['context_cache_default_enabled'] = False
            record['cache_usage_reporting'] = 'unknown'
            record['notes'] = 'Legacy OpenAI text completions models do not support tools, prompt caching, or reasoning controls.'
            records.append(record)
            continue

        record['tool_calls'] = tool_support
        record['tool_choice_required'] = tool_support
        record['assistant_tool_followup'] = 'none'
        record['reasoning_output'] = 'optional' if reasoning_supported and surface == 'responses' else 'unsupported'
        record['reasoning_activation'] = 'openai_reasoning_effort' if reasoning_supported else 'unavailable'
        record['context_cache_modes'] = ['passive'] if prompt_cache_supported else []
        record['context_cache_default_enabled'] = prompt_cache_supported
        record['cache_usage_reporting'] = 'standard_usage' if prompt_cache_supported else 'unknown'

        notes: list[str] = []
        if response_only:
            notes.append('Model docs mark this family as Responses API only.')
        elif chat_only:
            notes.append('Model docs describe this family for Chat Completions flows.')
        if reasoning_supported:
            if surface == 'responses':
                notes.append('This reasoning-capable family uses OpenAI reasoning.effort on the Responses API.')
            else:
                notes.append('This reasoning-capable family can use reasoning.effort, but chat completions does not expose reasoning items.')
        if prompt_cache_supported:
            notes.append('Prompt caching applies automatically on recent OpenAI models and reports through standard cached_tokens usage details.')
        if notes:
            record['notes'] = ' '.join(notes)

        records.append(record)

    return records


def build_behavior_records(provider_id: str, model_id: str, model: dict, surfaces: list[str]) -> list[OrderedDict[str, object]]:
    if provider_id == 'openai':
        return build_openai_behavior_records(model_id, model, surfaces)
    return []


def find_model_starts(lines: list[str]) -> list[tuple[str, int]]:
    starts: list[tuple[str, int]] = []
    pattern = re.compile(r'^\[models\."(.+)"\]$')
    for idx, line in enumerate(lines):
        match = pattern.match(line.strip())
        if match:
            starts.append((match.group(1), idx))
    return starts


def find_top_insert_index(lines: list[str], start: int, end: int, model_id: str) -> int:
    subtable = f'[models.{toml_quote(model_id)}.'
    array_table = f'[[models.{toml_quote(model_id)}.'
    for idx in range(start + 1, end):
        stripped = lines[idx].strip()
        if stripped.startswith(subtable) or stripped.startswith(array_table) or stripped.startswith('[models."'):
            return idx
    return end


def current_api_line_index(lines: list[str], start: int, end: int) -> int | None:
    for idx in range(start + 1, end):
        stripped = lines[idx].strip()
        if stripped.startswith('api_surfaces = '):
            return idx
        if stripped.startswith('[models."') or stripped.startswith('[[models."'):
            break
    return None


def has_api_record(model: dict) -> bool:
    records = model.get('records') or []
    return any(isinstance(r, dict) and ('api_surface' in r or 'api_surfaces' in r) for r in records)


def has_behavior_record(model: dict) -> bool:
    records = model.get('records') or []
    return any(isinstance(r, dict) and r.get('table_kind') == 'behavior' for r in records)


def find_record_blocks(lines: list[str], start: int, end: int, model_id: str) -> list[tuple[int, int]]:
    header = f'[[models.{toml_quote(model_id)}.records]]'
    blocks: list[tuple[int, int]] = []
    idx = start + 1
    while idx < end:
        stripped = lines[idx].strip()
        if stripped == header:
            block_start = idx
            idx += 1
            while idx < end:
                nxt = lines[idx].strip()
                if nxt.startswith('[[models."') or nxt.startswith('[models."'):
                    break
                idx += 1
            blocks.append((block_start, idx))
        else:
            idx += 1
    return blocks


def parse_record_block(lines: list[str], block_start: int, block_end: int) -> tuple[dict[str, str], dict[str, int]]:
    values: dict[str, str] = {}
    indexes: dict[str, int] = {}
    for idx in range(block_start + 1, block_end):
        match = QUOTED_STRING_RE.match(lines[idx].strip())
        if not match:
            continue
        key, value = match.groups()
        values[key] = value
        indexes[key] = idx
    return values, indexes


def preferred_endpoint_insert_index(block_end: int, indexes: dict[str, int]) -> int:
    if 'api_surface' in indexes:
        return indexes['api_surface'] + 1
    if 'section' in indexes:
        return indexes['section'] + 1
    if 'source_page' in indexes:
        return indexes['source_page'] + 1
    if 'notes' in indexes:
        return indexes['notes']
    return block_end


def repair_existing_api_blocks(lines: list[str], provider_id: str, model_id: str, model: dict, start: int, end: int) -> int:
    repaired = 0
    blocks = find_record_blocks(lines, start, end, model_id)
    for block_start, block_end in reversed(blocks):
        values, indexes = parse_record_block(lines, block_start, block_end)
        if values.get('table_kind') != 'api_reference':
            continue
        surface = values.get('api_surface')
        if not surface:
            continue
        desired_endpoint = format_endpoint(endpoint_for_surface(provider_id, surface, model_id, model), model_id, model)
        desired_source_url = official_source_url_for(provider_id, surface, model_id, model)
        desired_source_page = source_page_for(provider_id, surface, model)
        desired_notes = notes_for(provider_id, surface, model_id, model, inferred=False)

        changed = False
        pending_insertions: list[tuple[int, str]] = []

        if 'endpoint' in indexes and desired_endpoint and values.get('endpoint') != desired_endpoint:
            lines[indexes['endpoint']] = f'endpoint = {toml_quote(desired_endpoint)}'
            changed = True
        elif 'endpoint' not in indexes and desired_endpoint:
            pending_insertions.append((preferred_endpoint_insert_index(block_end, indexes), f'endpoint = {toml_quote(desired_endpoint)}'))

        if desired_source_url and values.get('source_url') != desired_source_url:
            if 'source_url' in indexes:
                lines[indexes['source_url']] = f'source_url = {toml_quote(desired_source_url)}'
            else:
                pending_insertions.append((block_start + 1, f'source_url = {toml_quote(desired_source_url)}'))
            changed = True

        if values.get('source_page') != desired_source_page:
            if 'source_page' in indexes:
                lines[indexes['source_page']] = f'source_page = {toml_quote(desired_source_page)}'
            else:
                pending_insertions.append((block_start + 1, f'source_page = {toml_quote(desired_source_page)}'))
            changed = True

        if values.get('notes') != desired_notes:
            if 'notes' in indexes:
                lines[indexes['notes']] = f'notes = {toml_quote(desired_notes)}'
            else:
                pending_insertions.append((block_end, f'notes = {toml_quote(desired_notes)}'))
            changed = True

        if pending_insertions:
            for insert_at, line in sorted(pending_insertions, key=lambda item: item[0], reverse=True):
                lines[insert_at:insert_at] = [line]
            changed = True

        if changed:
            repaired += 1
    return repaired


def find_record_blocks_of_kind(lines: list[str], start: int, end: int, model_id: str, table_kind: str) -> list[tuple[int, int]]:
    matches: list[tuple[int, int]] = []
    for block_start, block_end in find_record_blocks(lines, start, end, model_id):
        values, _ = parse_record_block(lines, block_start, block_end)
        if values.get('table_kind') == table_kind:
            matches.append((block_start, block_end))
    return matches


def remove_record_blocks(lines: list[str], blocks: list[tuple[int, int]]) -> int:
    removed_lines = 0
    for block_start, block_end in reversed(blocks):
        delete_start = block_start
        if delete_start > 0 and not lines[delete_start - 1].strip():
            delete_start -= 1
        removed_lines += block_end - delete_start
        del lines[delete_start:block_end]
    return removed_lines


def sync_behavior_blocks(lines: list[str], provider_id: str, model_id: str, model: dict, surfaces: list[str], start: int, end: int) -> int:
    desired_records = build_behavior_records(provider_id, model_id, model, surfaces)
    existing_blocks = find_record_blocks_of_kind(lines, start, end, model_id, 'behavior')
    removed_lines = remove_record_blocks(lines, existing_blocks) if existing_blocks else 0
    end -= removed_lines

    if not desired_records:
        return 1 if existing_blocks else 0

    record_lines: list[str] = []
    if end > 0 and lines[end - 1].strip():
        record_lines.append('')
    for index, record in enumerate(desired_records):
        if index and record_lines and record_lines[-1] != '':
            record_lines.append('')
        record_lines.extend(render_record(model_id, record))
        record_lines.append('')
    if record_lines and record_lines[-1] == '':
        record_lines.pop()
    lines[end:end] = record_lines
    return 1


def enrich_file(path: Path) -> tuple[int, int, int, int]:
    raw_text = path.read_text(encoding='utf-8')
    data = toml.loads(raw_text)
    provider_id = data['provider']['id']
    models = data.get('models', {})
    lines = raw_text.splitlines()
    starts = find_model_starts(lines)
    start_lookup = {model_id: idx for model_id, idx in starts}
    order = [model_id for model_id, _ in starts]
    insertions = 0
    updated_surfaces = 0
    repaired_blocks = 0
    synced_behaviors = 0

    for pos in range(len(order) - 1, -1, -1):
        model_id = order[pos]
        model = models[model_id]
        start = start_lookup[model_id]
        end = start_lookup[order[pos + 1]] if pos + 1 < len(order) else len(lines)

        surfaces = list(model.get('api_surfaces') or [])
        inferred_surfaces = False
        if provider_id in {'openai', 'google', 'anthropic', 'kimi'}:
            new_surfaces = infer_surfaces(provider_id, model_id, model)
            inferred_surfaces = True
        elif provider_id == 'qianfan' and not surfaces:
            new_surfaces = infer_surfaces(provider_id, model_id, model)
            inferred_surfaces = True
        else:
            new_surfaces = surfaces

        if (not surfaces) or surfaces != new_surfaces:
            api_line_idx = current_api_line_index(lines, start, end)
            new_line = f'api_surfaces = {toml_array(new_surfaces)}'
            if api_line_idx is not None:
                lines[api_line_idx] = new_line
            else:
                insert_at = find_top_insert_index(lines, start, end, model_id)
                lines[insert_at:insert_at] = [new_line]
                end += 1
            updated_surfaces += 1
            insertions += 1
            surfaces = new_surfaces

        repaired_blocks += repair_existing_api_blocks(lines, provider_id, model_id, model, start, end)

        if not has_api_record(model):
            records = build_records(provider_id, model_id, model, surfaces, inferred_surfaces)
            if records:
                record_lines: list[str] = []
                if end > 0 and lines[end - 1].strip():
                    record_lines.append('')
                for index, record in enumerate(records):
                    if index and record_lines and record_lines[-1] != '':
                        record_lines.append('')
                    record_lines.extend(render_record(model_id, record))
                    record_lines.append('')
                if record_lines and record_lines[-1] == '':
                    record_lines.pop()
                lines[end:end] = record_lines
                insertions += 1

        if provider_id == 'openai':
            synced_behaviors += sync_behavior_blocks(lines, provider_id, model_id, model, surfaces, start, end)

    new_text = '\n'.join(lines) + '\n'
    path.write_text(new_text, encoding='utf-8')
    write_json_sidecar(path)
    return insertions, updated_surfaces, repaired_blocks, synced_behaviors


if __name__ == '__main__':
    for name in TARGET_FILES:
        path = CATALOG_DIR / name
        insertions, updated_surfaces, repaired_blocks, synced_behaviors = enrich_file(path)
        print(
            f'{path.name}: insertions={insertions} updated_surfaces={updated_surfaces} '
            f'repaired_blocks={repaired_blocks} synced_behaviors={synced_behaviors}'
        )
