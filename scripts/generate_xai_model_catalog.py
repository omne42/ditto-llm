#!/usr/bin/env python3
from __future__ import annotations

import argparse
import datetime as dt
import sys
from collections import OrderedDict
from pathlib import Path

from provider_model_catalog_json import write_json_sidecar

MODELS_PRICING_URL = 'https://docs.x.ai/developers/models'
OVERVIEW_URL = 'https://docs.x.ai/docs/overview'
CHAT_API_URL = 'https://docs.x.ai/api/operation/operation-create-chat-completion'
IMAGE_API_URL = 'https://docs.x.ai/api/operation/operation-createimage'
VIDEO_API_URL = 'https://docs.x.ai/api/operation/operation-createvideo'
GROK_4_URL = 'https://docs.x.ai/docs/models/grok-4'
GROK_4_FAST_REASONING_URL = 'https://docs.x.ai/docs/models/grok-4-fast-reasoning'
GROK_3_FAST_URL = 'https://docs.x.ai/docs/models/grok-3-fast'
GROK_CODE_FAST_1_URL = 'https://docs.x.ai/docs/models/grok-code-fast-1'
GROK_IMAGE_URL = 'https://docs.x.ai/docs/models/grok-imagine-image'
GROK_VIDEO_URL = 'https://docs.x.ai/docs/models/grok-imagine-video'
DEFAULT_OUTPUT = Path(__file__).resolve().parents[1] / 'catalog' / 'provider_models' / 'xai.toml'
BASE_URL = 'https://api.x.ai/v1'


def current_models() -> OrderedDict[str, dict]:
    return OrderedDict(
        [
            (
                'grok-4-0709',
                {
                    'source_url': GROK_4_URL,
                    'source_urls': [GROK_4_URL, MODELS_PRICING_URL, CHAT_API_URL],
                    'display_name': 'Grok 4',
                    'status': 'active',
                    'vendor': 'xai',
                    'api_surfaces': ['chat.completion'],
                    'categories': ['文本模型', '推理模型', '视觉模型', '代码模型'],
                    'aliases': ['grok-4'],
                    'summary': 'Powerful frontier model designed for real-world utility with native tool use, search, vision, and strong coding performance.',
                    'context_window_tokens': 256000,
                    'max_output_tokens': 16000,
                    'input_modalities': ['text', 'image'],
                    'output_modalities': ['text'],
                    'supported_features': ['vision', 'structured_outputs', 'tool_use', 'web_search', 'coding'],
                    'pricing_input_usd_per_million': '3.00',
                    'pricing_output_usd_per_million': '15.00',
                    'records': [
                        OrderedDict(
                            table_kind='model_page',
                            source_url=GROK_4_URL,
                            source_page='grok_4',
                            section='Model summary',
                            api_model_name='grok-4-0709',
                            model_alias='grok-4',
                            image_support='true',
                            structured_outputs='true',
                            notes='The Grok 4 model page lists the API model name, latest alias, image support, structured outputs, and pricing.',
                        ),
                        OrderedDict(
                            table_kind='api_reference',
                            source_url=CHAT_API_URL,
                            source_page='create_chat_completion',
                            section='Create Chat Completion',
                            api_surface='chat.completion',
                            endpoint=f'{BASE_URL}/chat/completions',
                            notes='xAI exposes Grok text and multimodal chat models through the Chat Completions API.',
                        ),
                        OrderedDict(
                            table_kind='alias_mapping',
                            source_url=MODELS_PRICING_URL,
                            source_page='models_and_pricing',
                            section='Model Aliases',
                            alias='grok-4',
                            resolved_model='grok-4-0709',
                        ),
                    ],
                },
            ),
            (
                'grok-4-fast-reasoning-0709',
                {
                    'source_url': GROK_4_FAST_REASONING_URL,
                    'source_urls': [GROK_4_FAST_REASONING_URL, MODELS_PRICING_URL, CHAT_API_URL],
                    'display_name': 'Grok 4 Fast Reasoning',
                    'status': 'active',
                    'vendor': 'xai',
                    'api_surfaces': ['chat.completion'],
                    'categories': ['文本模型', '推理模型', '长上下文模型'],
                    'aliases': ['grok-4-fast-reasoning'],
                    'summary': 'Reasoning model optimized for high-throughput agentic and long-context workloads with a 2M-token context window.',
                    'context_window_tokens': 2000000,
                    'max_output_tokens': 64000,
                    'input_modalities': ['text'],
                    'output_modalities': ['text'],
                    'supported_features': ['structured_outputs', 'reasoning', 'long_context'],
                    'pricing_input_usd_per_million': '0.20',
                    'pricing_output_usd_per_million': '0.50',
                    'records': [
                        OrderedDict(
                            table_kind='model_page',
                            source_url=GROK_4_FAST_REASONING_URL,
                            source_page='grok_4_fast_reasoning',
                            section='Model summary',
                            api_model_name='grok-4-fast-reasoning-0709',
                            model_alias='grok-4-fast-reasoning',
                            image_support='false',
                            structured_outputs='true',
                            notes='The Grok 4 Fast Reasoning page documents the API model id, alias, 2M context window, and token pricing.',
                        ),
                        OrderedDict(
                            table_kind='api_reference',
                            source_url=CHAT_API_URL,
                            source_page='create_chat_completion',
                            section='Create Chat Completion',
                            api_surface='chat.completion',
                            endpoint=f'{BASE_URL}/chat/completions',
                            notes='Reasoning-capable Grok text models are served through the Chat Completions API.',
                        ),
                        OrderedDict(
                            table_kind='alias_mapping',
                            source_url=MODELS_PRICING_URL,
                            source_page='models_and_pricing',
                            section='Model Aliases',
                            alias='grok-4-fast-reasoning',
                            resolved_model='grok-4-fast-reasoning-0709',
                        ),
                    ],
                },
            ),
            (
                'grok-3-fast-20251025',
                {
                    'source_url': GROK_3_FAST_URL,
                    'source_urls': [GROK_3_FAST_URL, MODELS_PRICING_URL, CHAT_API_URL],
                    'display_name': 'Grok 3 Fast',
                    'status': 'active',
                    'vendor': 'xai',
                    'api_surfaces': ['chat.completion'],
                    'categories': ['文本模型', '低延迟模型'],
                    'aliases': ['grok-3-fast'],
                    'summary': 'Low-latency Grok model for day-to-day conversational tasks, optimized for responsiveness and throughput.',
                    'context_window_tokens': 2000000,
                    'max_output_tokens': 16000,
                    'input_modalities': ['text'],
                    'output_modalities': ['text'],
                    'supported_features': ['structured_outputs', 'low_latency'],
                    'pricing_input_usd_per_million': '5.00',
                    'pricing_output_usd_per_million': '25.00',
                    'records': [
                        OrderedDict(
                            table_kind='model_page',
                            source_url=GROK_3_FAST_URL,
                            source_page='grok_3_fast',
                            section='Model summary',
                            api_model_name='grok-3-fast-20251025',
                            model_alias='grok-3-fast',
                            image_support='false',
                            structured_outputs='true',
                            notes='The Grok 3 Fast page documents the API model id, latest alias, 2M context window, and token pricing.',
                        ),
                        OrderedDict(
                            table_kind='api_reference',
                            source_url=CHAT_API_URL,
                            source_page='create_chat_completion',
                            section='Create Chat Completion',
                            api_surface='chat.completion',
                            endpoint=f'{BASE_URL}/chat/completions',
                            notes='Grok 3 Fast is served through the Chat Completions API.',
                        ),
                        OrderedDict(
                            table_kind='alias_mapping',
                            source_url=MODELS_PRICING_URL,
                            source_page='models_and_pricing',
                            section='Model Aliases',
                            alias='grok-3-fast',
                            resolved_model='grok-3-fast-20251025',
                        ),
                    ],
                },
            ),
            (
                'grok-code-fast-1-0825',
                {
                    'source_url': GROK_CODE_FAST_1_URL,
                    'source_urls': [GROK_CODE_FAST_1_URL, MODELS_PRICING_URL, CHAT_API_URL],
                    'display_name': 'Grok Code Fast 1',
                    'status': 'active',
                    'vendor': 'xai',
                    'api_surfaces': ['chat.completion'],
                    'categories': ['代码模型', '文本模型'],
                    'aliases': ['grok-code-fast-1'],
                    'summary': 'Fast specialized coding model for autocomplete, edit suggestions, and agentic software development tasks.',
                    'context_window_tokens': 131072,
                    'input_modalities': ['text'],
                    'output_modalities': ['text'],
                    'supported_features': ['coding', 'code_completion', 'edit_suggestions', 'agentic_development'],
                    'pricing_input_usd_per_million': '0.20',
                    'pricing_output_usd_per_million': '1.50',
                    'records': [
                        OrderedDict(
                            table_kind='model_page',
                            source_url=GROK_CODE_FAST_1_URL,
                            source_page='grok_code_fast_1',
                            section='Model summary',
                            api_model_name='grok-code-fast-1-0825',
                            model_alias='grok-code-fast-1',
                            notes='The Grok Code Fast 1 page documents the API model id, alias, and coding-focused positioning.',
                        ),
                        OrderedDict(
                            table_kind='pricing_table',
                            source_url=MODELS_PRICING_URL,
                            source_page='models_and_pricing',
                            section='Models and Pricing',
                            api_model_name='grok-code-fast-1-0825',
                            context_window_tokens='131072',
                            input_pricing_usd_per_million='0.20',
                            output_pricing_usd_per_million='1.50',
                            notes='The models and pricing table provides the 131,072-token context window and token pricing for Grok Code Fast 1.',
                        ),
                        OrderedDict(
                            table_kind='api_reference',
                            source_url=CHAT_API_URL,
                            source_page='create_chat_completion',
                            section='Create Chat Completion',
                            api_surface='chat.completion',
                            endpoint=f'{BASE_URL}/chat/completions',
                            notes='Coding-oriented Grok text models are exposed via the Chat Completions API.',
                        ),
                        OrderedDict(
                            table_kind='alias_mapping',
                            source_url=MODELS_PRICING_URL,
                            source_page='models_and_pricing',
                            section='Model Aliases',
                            alias='grok-code-fast-1',
                            resolved_model='grok-code-fast-1-0825',
                        ),
                    ],
                },
            ),
            (
                'grok-2-image-1212',
                {
                    'source_url': GROK_IMAGE_URL,
                    'source_urls': [GROK_IMAGE_URL, MODELS_PRICING_URL, IMAGE_API_URL],
                    'display_name': 'Grok 2 Image',
                    'status': 'active',
                    'vendor': 'xai',
                    'api_surfaces': ['image.generation'],
                    'categories': ['图像生成模型'],
                    'summary': 'Text-to-image model tuned for strong prompt adherence and high-quality generations without a fixed style.',
                    'max_input_tokens': 4096,
                    'max_output_images': 1,
                    'input_modalities': ['text'],
                    'output_modalities': ['image'],
                    'supported_features': ['prompt_adherence', 'single_image_output'],
                    'pricing_per_image_usd': '0.07',
                    'records': [
                        OrderedDict(
                            table_kind='model_page',
                            source_url=GROK_IMAGE_URL,
                            source_page='grok_imagine_image',
                            section='Model summary',
                            api_model_name='grok-2-image-1212',
                            notes='The Grok image generation page documents the API model name, prompt-only input, single-image output, and per-image pricing.',
                        ),
                        OrderedDict(
                            table_kind='api_reference',
                            source_url=IMAGE_API_URL,
                            source_page='create_image',
                            section='Create Image',
                            api_surface='image.generation',
                            endpoint=f'{BASE_URL}/images/generations',
                            notes='xAI image generation models are exposed through the Create Image endpoint.',
                        ),
                    ],
                },
            ),
            (
                'grok-imagine-2',
                {
                    'source_url': GROK_VIDEO_URL,
                    'source_urls': [GROK_VIDEO_URL, MODELS_PRICING_URL, VIDEO_API_URL],
                    'display_name': 'Grok Imagine 2',
                    'status': 'active',
                    'vendor': 'xai',
                    'api_surfaces': ['video.generation'],
                    'categories': ['视频生成模型'],
                    'summary': 'Text-and-image conditioned video generation model for high-quality video creation.',
                    'max_input_images': 7,
                    'supported_output_lengths_seconds': ['5', '15', '30'],
                    'input_modalities': ['text', 'image'],
                    'output_modalities': ['video'],
                    'supported_features': ['image_conditioning', 'multi_image_input'],
                    'pricing_5_seconds_cents': '35.00',
                    'pricing_15_seconds_cents': '50.00',
                    'pricing_30_seconds_cents': '100.00',
                    'records': [
                        OrderedDict(
                            table_kind='model_page',
                            source_url=GROK_VIDEO_URL,
                            source_page='grok_imagine_video',
                            section='Model summary',
                            api_model_name='grok-imagine-2',
                            notes='The Grok video generation page documents the API model name, text/image inputs, supported output lengths, and pricing in cents.',
                        ),
                        OrderedDict(
                            table_kind='api_reference',
                            source_url=VIDEO_API_URL,
                            source_page='create_video',
                            section='Create Video',
                            api_surface='video.generation',
                            endpoint=f'{BASE_URL}/video/generations',
                            notes='xAI video generation models are exposed through the Create Video endpoint.',
                        ),
                    ],
                },
            ),
        ]
    )


def alias_only_models() -> OrderedDict[str, dict]:
    return OrderedDict(
        [
            (
                'grok-2-latest',
                {
                    'source_url': MODELS_PRICING_URL,
                    'display_name': 'grok-2-latest',
                    'status': 'active',
                    'vendor': 'xai',
                    'api_surfaces': ['chat.completion'],
                    'categories': ['模型别名', '文本模型'],
                    'summary': 'Official latest alias listed in xAI Models and Pricing.',
                    'resolved_model_version': 'grok-2-1212',
                    'records': [
                        OrderedDict(
                            table_kind='alias_mapping',
                            source_url=MODELS_PRICING_URL,
                            source_page='models_and_pricing',
                            section='Model Aliases',
                            alias='grok-2-latest',
                            resolved_model='grok-2-1212',
                        ),
                        OrderedDict(
                            table_kind='api_reference',
                            source_url=CHAT_API_URL,
                            source_page='create_chat_completion',
                            section='Create Chat Completion',
                            api_surface='chat.completion',
                            endpoint=f'{BASE_URL}/chat/completions',
                            notes='Alias-only Grok text models still resolve through the Chat Completions API.',
                        ),
                    ],
                },
            ),
            (
                'grok-2-vision-latest',
                {
                    'source_url': MODELS_PRICING_URL,
                    'display_name': 'grok-2-vision-latest',
                    'status': 'active',
                    'vendor': 'xai',
                    'api_surfaces': ['chat.completion'],
                    'categories': ['模型别名', '视觉模型'],
                    'summary': 'Official latest alias listed in xAI Models and Pricing.',
                    'resolved_model_version': 'grok-2-vision-1212',
                    'records': [
                        OrderedDict(
                            table_kind='alias_mapping',
                            source_url=MODELS_PRICING_URL,
                            source_page='models_and_pricing',
                            section='Model Aliases',
                            alias='grok-2-vision-latest',
                            resolved_model='grok-2-vision-1212',
                        ),
                        OrderedDict(
                            table_kind='api_reference',
                            source_url=CHAT_API_URL,
                            source_page='create_chat_completion',
                            section='Create Chat Completion',
                            api_surface='chat.completion',
                            endpoint=f'{BASE_URL}/chat/completions',
                            notes='Alias-only Grok vision models are invoked through the Chat Completions API.',
                        ),
                    ],
                },
            ),
            (
                'grok-beta',
                {
                    'source_url': MODELS_PRICING_URL,
                    'display_name': 'grok-beta',
                    'status': 'active',
                    'vendor': 'xai',
                    'api_surfaces': ['chat.completion'],
                    'categories': ['模型别名', '文本模型'],
                    'summary': 'Official Grok beta alias still listed in xAI Models and Pricing.',
                    'resolved_model_version': 'grok-beta',
                    'records': [
                        OrderedDict(
                            table_kind='alias_mapping',
                            source_url=MODELS_PRICING_URL,
                            source_page='models_and_pricing',
                            section='Model Aliases',
                            alias='grok-beta',
                            resolved_model='grok-beta',
                        ),
                        OrderedDict(
                            table_kind='api_reference',
                            source_url=CHAT_API_URL,
                            source_page='create_chat_completion',
                            section='Create Chat Completion',
                            api_surface='chat.completion',
                            endpoint=f'{BASE_URL}/chat/completions',
                            notes='Legacy Grok beta aliases remain documented as chat-capable model ids.',
                        ),
                    ],
                },
            ),
            (
                'grok-vision-beta',
                {
                    'source_url': MODELS_PRICING_URL,
                    'display_name': 'grok-vision-beta',
                    'status': 'active',
                    'vendor': 'xai',
                    'api_surfaces': ['chat.completion'],
                    'categories': ['模型别名', '视觉模型'],
                    'summary': 'Official Grok vision beta alias still listed in xAI Models and Pricing.',
                    'resolved_model_version': 'grok-vision-beta',
                    'records': [
                        OrderedDict(
                            table_kind='alias_mapping',
                            source_url=MODELS_PRICING_URL,
                            source_page='models_and_pricing',
                            section='Model Aliases',
                            alias='grok-vision-beta',
                            resolved_model='grok-vision-beta',
                        ),
                        OrderedDict(
                            table_kind='api_reference',
                            source_url=CHAT_API_URL,
                            source_page='create_chat_completion',
                            section='Create Chat Completion',
                            api_surface='chat.completion',
                            endpoint=f'{BASE_URL}/chat/completions',
                            notes='Legacy Grok beta vision aliases are invoked through the Chat Completions API.',
                        ),
                    ],
                },
            ),
            (
                'grok-3',
                {
                    'source_url': MODELS_PRICING_URL,
                    'display_name': 'grok-3',
                    'status': 'active',
                    'vendor': 'xai',
                    'api_surfaces': ['chat.completion'],
                    'categories': ['模型别名', '文本模型'],
                    'summary': 'Official latest alias listed in xAI Models and Pricing.',
                    'resolved_model_version': 'grok-3-20250217',
                    'records': [
                        OrderedDict(
                            table_kind='alias_mapping',
                            source_url=MODELS_PRICING_URL,
                            source_page='models_and_pricing',
                            section='Model Aliases',
                            alias='grok-3',
                            resolved_model='grok-3-20250217',
                        ),
                        OrderedDict(
                            table_kind='api_reference',
                            source_url=CHAT_API_URL,
                            source_page='create_chat_completion',
                            section='Create Chat Completion',
                            api_surface='chat.completion',
                            endpoint=f'{BASE_URL}/chat/completions',
                            notes='Alias-only Grok 3 requests are routed through the Chat Completions API.',
                        ),
                    ],
                },
            ),
            (
                'grok-3-mini',
                {
                    'source_url': MODELS_PRICING_URL,
                    'display_name': 'grok-3-mini',
                    'status': 'active',
                    'vendor': 'xai',
                    'api_surfaces': ['chat.completion'],
                    'categories': ['模型别名', '推理模型'],
                    'summary': 'Official latest alias listed in xAI Models and Pricing.',
                    'resolved_model_version': 'grok-3-mini-20250217',
                    'records': [
                        OrderedDict(
                            table_kind='alias_mapping',
                            source_url=MODELS_PRICING_URL,
                            source_page='models_and_pricing',
                            section='Model Aliases',
                            alias='grok-3-mini',
                            resolved_model='grok-3-mini-20250217',
                        ),
                        OrderedDict(
                            table_kind='api_reference',
                            source_url=CHAT_API_URL,
                            source_page='create_chat_completion',
                            section='Create Chat Completion',
                            api_surface='chat.completion',
                            endpoint=f'{BASE_URL}/chat/completions',
                            notes='Alias-only Grok 3 Mini requests are routed through the Chat Completions API.',
                        ),
                    ],
                },
            ),
        ]
    )


def all_models() -> OrderedDict[str, dict]:
    models = OrderedDict()
    models.update(current_models())
    models.update(alias_only_models())
    return models


def toml_quote(value: str) -> str:
    return '"' + value.replace('\\', '\\\\').replace('"', '\\"') + '"'


def toml_array(values: list[str]) -> str:
    return '[' + ', '.join(toml_quote(value) for value in values) + ']'


def write_key_value(lines: list[str], key: str, value) -> None:
    if isinstance(value, list):
        lines.append(f'{key} = {toml_array(value)}')
    elif isinstance(value, bool):
        lines.append(f"{key} = {'true' if value else 'false'}")
    elif isinstance(value, int):
        lines.append(f'{key} = {value}')
    else:
        lines.append(f'{key} = {toml_quote(str(value))}')


def write_record(lines: list[str], path: str, record: OrderedDict[str, object]) -> None:
    lines.append(f'[[{path}]]')
    for key, value in record.items():
        write_key_value(lines, key, value)
    lines.append('')


def render_toml(models: OrderedDict[str, dict]) -> str:
    now = dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat().replace('+00:00', 'Z')
    lines = [
        '# Generated from official xAI / Grok docs.',
        '# Edit via scripts/generate_xai_model_catalog.py.',
        '# Sources:',
        f'# - {MODELS_PRICING_URL}',
        f'# - {OVERVIEW_URL}',
        f'# - {CHAT_API_URL}',
        f'# - {IMAGE_API_URL}',
        f'# - {VIDEO_API_URL}',
        f'# - {GROK_4_URL}',
        f'# - {GROK_4_FAST_REASONING_URL}',
        f'# - {GROK_3_FAST_URL}',
        f'# - {GROK_CODE_FAST_1_URL}',
        f'# - {GROK_IMAGE_URL}',
        f'# - {GROK_VIDEO_URL}',
        f'# Generated at: {now}',
        '',
        '[provider]',
        'id = "xai"',
        'display_name = "xAI / Grok"',
        f'base_url = {toml_quote(BASE_URL)}',
        'protocol = "openai"',
        f'source_url = {toml_quote(MODELS_PRICING_URL)}',
        '',
        '[provider.auth]',
        'type = "api_key_env"',
        'keys = ["XAI_API_KEY"]',
        '',
    ]

    for model_id, data in models.items():
        lines.append(f'[models.{toml_quote(model_id)}]')
        write_key_value(lines, 'source_url', data['source_url'])
        if 'source_urls' in data and len(data['source_urls']) > 1:
            write_key_value(lines, 'source_urls', data['source_urls'])
        write_key_value(lines, 'display_name', data['display_name'])
        write_key_value(lines, 'status', data['status'])
        write_key_value(lines, 'vendor', data['vendor'])
        write_key_value(lines, 'api_surfaces', data['api_surfaces'])
        write_key_value(lines, 'categories', data['categories'])
        if 'aliases' in data:
            write_key_value(lines, 'aliases', data['aliases'])
        write_key_value(lines, 'summary', data['summary'])
        for key in [
            'resolved_model_version',
            'context_window_tokens',
            'max_input_tokens',
            'max_output_tokens',
            'max_output_images',
            'max_input_images',
            'pricing_input_usd_per_million',
            'pricing_output_usd_per_million',
            'pricing_per_image_usd',
            'pricing_5_seconds_cents',
            'pricing_15_seconds_cents',
            'pricing_30_seconds_cents',
        ]:
            if key in data:
                write_key_value(lines, key, data[key])
        for key in ['input_modalities', 'output_modalities', 'supported_features', 'supported_output_lengths_seconds']:
            if key in data:
                write_key_value(lines, key, data[key])
        lines.append('')
        record_path = f'models.{toml_quote(model_id)}.records'
        for record in data['records']:
            write_record(lines, record_path, record)
    return '\n'.join(lines).rstrip() + '\n'


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description='Generate xAI / Grok provider model catalog')
    parser.add_argument('--output', type=Path, default=DEFAULT_OUTPUT, help='Output TOML path')
    args = parser.parse_args(argv)

    models = all_models()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(render_toml(models), encoding='utf-8')
    write_json_sidecar(args.output)
    print(f'wrote {len(models)} models to {args.output}')
    return 0


if __name__ == '__main__':
    sys.exit(main())
