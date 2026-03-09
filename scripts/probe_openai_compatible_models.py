#!/usr/bin/env python3
from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import time
import uuid
from pathlib import Path
from typing import Any

import requests

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_CATALOG = ROOT / 'catalog' / 'provider_models' / 'openai.json'
DEFAULT_OUTPUT = ROOT / 'tmp' / 'openai_compatible_probe.json'
DEFAULT_TIMEOUT = 180
DEFAULT_MAX_ATTEMPTS = 3
STOP_RETRY_ERROR_CODES = {
    'OperationNotSupported',
    'integer_below_min_value',
    'invalid_type',
    'model_not_found',
    'unsupported_parameter',
    'unsupported_value',
}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description='Probe OpenAI-compatible upstreams with safer defaults, tool calls, and multi-turn cache checks.'
    )
    parser.add_argument('--base-url', required=True, help='Upstream OpenAI-compatible base URL, for example https://yunwu.ai/v1')
    parser.add_argument('--api-key', help='Bearer API key. Defaults to the env var selected by --env-key.')
    parser.add_argument('--env-key', default='OPENAI_API_KEY', help='Env var to load when --api-key is omitted.')
    parser.add_argument('--catalog', type=Path, default=DEFAULT_CATALOG, help='Local OpenAI provider catalog JSON.')
    parser.add_argument('--output', type=Path, default=DEFAULT_OUTPUT, help='Report JSON output path.')
    parser.add_argument('--models', nargs='+', required=True, help='Model ids to probe.')
    parser.add_argument('--max-attempts', type=int, default=DEFAULT_MAX_ATTEMPTS, help='HTTP attempts per availability probe.')
    parser.add_argument('--timeout', type=int, default=DEFAULT_TIMEOUT, help='Per-request timeout in seconds.')
    parser.add_argument('--availability-only', action='store_true', help='Only run basic availability probes.')
    parser.add_argument('--include-tools', action='store_true', help='Run tool-calling probes on the preferred successful surface.')
    parser.add_argument('--include-cache', action='store_true', help='Run fresh-nonce multi-turn cache probes on the preferred successful surface.')
    parser.add_argument('--extra-chat-models', nargs='*', default=[], help='Additionally try chat.completion for these models even when the official catalog marks them responses-only.')
    return parser.parse_args()


def load_api_key(args: argparse.Namespace) -> str:
    if args.api_key:
        return args.api_key
    api_key = os.environ.get(args.env_key)
    if api_key:
        return api_key
    raise SystemExit(f'missing API key: provide --api-key or export {args.env_key}')


def load_catalog(path: Path) -> dict[str, Any]:
    with path.open() as handle:
        data = json.load(handle)
    return data.get('models', {})


def unique_preserve(values: list[str]) -> list[str]:
    seen: set[str] = set()
    result: list[str] = []
    for value in values:
        if value in seen:
            continue
        seen.add(value)
        result.append(value)
    return result


def ordered_surfaces(model: str, official_surfaces: list[str], extra_chat_models: set[str]) -> list[str]:
    surfaces = list(official_surfaces)
    if model in extra_chat_models and 'chat.completion' not in surfaces:
        surfaces.append('chat.completion')
    order = {'responses': 0, 'chat.completion': 1, 'completion.legacy': 2}
    return sorted(unique_preserve(surfaces), key=lambda surface: order.get(surface, 99))


def availability_payload(model: str, surface: str) -> dict[str, Any]:
    if surface == 'responses':
        return {
            'model': model,
            'input': 'Reply with OK only.',
            'max_output_tokens': 128,
        }
    if surface == 'chat.completion':
        return {
            'model': model,
            'messages': [
                {'role': 'user', 'content': 'Reply with OK only.'},
            ],
        }
    if surface == 'completion.legacy':
        return {
            'model': model,
            'prompt': 'Reply with OK only.',
            'max_tokens': 16,
        }
    raise ValueError(f'unsupported surface {surface}')


def response_path(surface: str) -> str:
    if surface == 'responses':
        return '/responses'
    if surface == 'chat.completion':
        return '/chat/completions'
    if surface == 'completion.legacy':
        return '/completions'
    raise ValueError(f'unsupported surface {surface}')


def maybe_error(body: Any) -> dict[str, Any] | None:
    if isinstance(body, dict) and isinstance(body.get('error'), dict):
        return body['error']
    return None


def should_stop_retry(http_status: int | None, body: Any) -> bool:
    if http_status == 200:
        return True
    error = maybe_error(body) or {}
    code = error.get('code')
    if code in STOP_RETRY_ERROR_CODES:
        return True
    if http_status is not None and 400 <= http_status < 500 and http_status != 429:
        return True
    return False


def do_request(
    session: requests.Session,
    *,
    base_url: str,
    headers: dict[str, str],
    path: str,
    payload: dict[str, Any],
    timeout: int,
) -> tuple[int | None, Any]:
    try:
        response = session.post(base_url + path, headers=headers, json=payload, timeout=timeout)
    except Exception as exc:
        return None, {'exception': repr(exc)}

    try:
        body = response.json()
    except Exception:
        body = {'_raw_text': response.text[:8000]}
    return response.status_code, body


def extract_text_from_response_items(body: dict[str, Any]) -> str:
    chunks: list[str] = []
    for item in body.get('output') or []:
        if item.get('type') != 'message':
            continue
        for content in item.get('content') or []:
            if content.get('type') in {'output_text', 'text'}:
                chunks.append(content.get('text') or '')
    return ''.join(chunks)


def summarize_body(surface: str, body: Any) -> dict[str, Any]:
    if not isinstance(body, dict):
        return {'raw': repr(body)}

    error = maybe_error(body)
    usage = body.get('usage') or {}
    summary: dict[str, Any] = {
        'object': body.get('object'),
        'status': body.get('status'),
        'error': error,
        'incomplete_details': body.get('incomplete_details'),
    }

    if surface == 'responses':
        function_calls = [item for item in body.get('output') or [] if item.get('type') == 'function_call']
        summary.update(
            {
                'text': extract_text_from_response_items(body),
                'cached_tokens': ((usage.get('input_tokens_details') or {}).get('cached_tokens')),
                'reasoning_tokens': ((usage.get('output_tokens_details') or {}).get('reasoning_tokens')),
                'total_tokens': usage.get('total_tokens'),
                'input_tokens': usage.get('input_tokens'),
                'output_tokens': usage.get('output_tokens'),
                'function_calls': [
                    {
                        'name': item.get('name'),
                        'arguments': item.get('arguments'),
                        'call_id': item.get('call_id'),
                    }
                    for item in function_calls
                ],
            }
        )
        return summary

    if surface == 'chat.completion':
        choice = (body.get('choices') or [{}])[0]
        message = choice.get('message') or {}
        summary.update(
            {
                'text': message.get('content'),
                'finish_reason': choice.get('finish_reason'),
                'cached_tokens': ((usage.get('prompt_tokens_details') or {}).get('cached_tokens')),
                'reasoning_tokens': ((usage.get('completion_tokens_details') or {}).get('reasoning_tokens')),
                'total_tokens': usage.get('total_tokens'),
                'prompt_tokens': usage.get('prompt_tokens'),
                'completion_tokens': usage.get('completion_tokens'),
                'tool_calls': message.get('tool_calls') or [],
            }
        )
        return summary

    if surface == 'completion.legacy':
        choice = (body.get('choices') or [{}])[0]
        summary.update(
            {
                'text': choice.get('text'),
                'finish_reason': choice.get('finish_reason'),
                'total_tokens': usage.get('total_tokens'),
                'prompt_tokens': usage.get('prompt_tokens'),
                'completion_tokens': usage.get('completion_tokens'),
            }
        )
        return summary

    return summary


def run_availability_probe(
    session: requests.Session,
    *,
    model: str,
    surfaces: list[str],
    base_url: str,
    headers: dict[str, str],
    timeout: int,
    max_attempts: int,
) -> list[dict[str, Any]]:
    results: list[dict[str, Any]] = []
    for surface in surfaces:
        payload = availability_payload(model, surface)
        tries: list[dict[str, Any]] = []
        for _ in range(max_attempts):
            http_status, body = do_request(
                session,
                base_url=base_url,
                headers=headers,
                path=response_path(surface),
                payload=payload,
                timeout=timeout,
            )
            tries.append(
                {
                    'http_status': http_status,
                    'summary': summarize_body(surface, body),
                }
            )
            if should_stop_retry(http_status, body):
                break
            time.sleep(1.5)
        results.append(
            {
                'surface': surface,
                'path': response_path(surface),
                'payload': payload,
                'tries': tries,
            }
        )
    return results


def first_success_surface(availability: list[dict[str, Any]]) -> str | None:
    for entry in availability:
        tries = entry.get('tries') or []
        if tries and tries[0].get('http_status') == 200:
            return entry['surface']
    return None


def tool_payload(model: str, surface: str) -> dict[str, Any]:
    if surface == 'responses':
        payload: dict[str, Any] = {
            'model': model,
            'input': 'Call the tool with city="Paris".',
            'max_output_tokens': 512 if model == 'gpt-5.1-codex-max' else 128,
            'tools': [
                {
                    'type': 'function',
                    'name': 'get_weather',
                    'description': 'Get weather by city',
                    'parameters': {
                        'type': 'object',
                        'properties': {'city': {'type': 'string'}},
                        'required': ['city'],
                        'additionalProperties': False,
                    },
                }
            ],
            'tool_choice': 'required',
        }
        return payload
    if surface == 'chat.completion':
        return {
            'model': model,
            'messages': [
                {'role': 'user', 'content': 'Call the tool with city="Paris".'},
            ],
            'tools': [
                {
                    'type': 'function',
                    'function': {
                        'name': 'get_weather',
                        'description': 'Get weather by city',
                        'parameters': {
                            'type': 'object',
                            'properties': {'city': {'type': 'string'}},
                            'required': ['city'],
                            'additionalProperties': False,
                        },
                    },
                }
            ],
            'tool_choice': 'required',
        }
    raise ValueError(f'tool probe unsupported for surface {surface}')


def run_tool_probe(
    session: requests.Session,
    *,
    model: str,
    surface: str,
    base_url: str,
    headers: dict[str, str],
    timeout: int,
) -> dict[str, Any]:
    payload = tool_payload(model, surface)
    http_status, body = do_request(
        session,
        base_url=base_url,
        headers=headers,
        path=response_path(surface),
        payload=payload,
        timeout=timeout,
    )
    summary = summarize_body(surface, body)
    success = False
    if surface == 'responses':
        success = bool(summary.get('function_calls'))
    elif surface == 'chat.completion':
        success = bool(summary.get('tool_calls')) and summary.get('finish_reason') == 'tool_calls'
    return {
        'surface': surface,
        'path': response_path(surface),
        'payload': payload,
        'http_status': http_status,
        'summary': summary,
        'tool_success': success,
    }


def cache_candidate_settings(model: str) -> list[dict[str, Any]]:
    if model == 'gpt-5.1-codex-max':
        return [
            {'max_output_tokens': 128},
            {'max_output_tokens': 512, 'reasoning': {'effort': 'medium'}},
            {'max_output_tokens': 512},
        ]
    if model.startswith(('gpt-5', 'o1', 'o3', 'o4')):
        return [
            {'max_output_tokens': 128},
            {'max_output_tokens': 128, 'reasoning': {'effort': 'minimal'}},
            {'max_output_tokens': 256, 'reasoning': {'effort': 'medium'}},
        ]
    return [{'max_output_tokens': 128}]


def build_cache_messages(nonce_prefix: str, round1_reply: str | None = None) -> tuple[str, str, list[dict[str, Any]]]:
    system = 'You are a terse assistant. Respect prior turns exactly and answer with only the requested short token.'
    round1_user = nonce_prefix + '\n\nRound1 task: reply with token R1 only.'
    round2_user = 'Round2 task: given the exact prior exchange, reply with token R2 only.'
    input_items = [
        {'role': 'system', 'content': system},
        {'role': 'user', 'content': round1_user},
    ]
    if round1_reply is not None:
        input_items.append({'role': 'assistant', 'content': round1_reply})
        input_items.append({'role': 'user', 'content': round2_user})
    return system, round1_user, input_items


def run_cache_probe(
    session: requests.Session,
    *,
    model: str,
    surface: str,
    base_url: str,
    headers: dict[str, str],
    timeout: int,
) -> dict[str, Any]:
    if surface not in {'responses', 'chat.completion'}:
        return {
            'surface': surface,
            'unsupported': True,
            'reason': 'cache probe only supports responses and chat.completion',
        }

    nonce_prefix = ((f'UNIQUE-{uuid.uuid4().hex} CACHE-CANDIDATE-LONG-PREFIX-OMEGA ') * 900).strip()
    attempts: list[dict[str, Any]] = []

    for settings in cache_candidate_settings(model):
        if surface == 'responses':
            _, _, round1_input = build_cache_messages(nonce_prefix)
            payload1: dict[str, Any] = {'model': model, 'input': round1_input, **settings}
            status1, body1 = do_request(
                session,
                base_url=base_url,
                headers=headers,
                path='/responses',
                payload=payload1,
                timeout=timeout,
            )
            summary1 = summarize_body(surface, body1)
            assistant_text = summary1.get('text')
            _, _, round2_input = build_cache_messages(nonce_prefix, assistant_text)
            payload2: dict[str, Any] = {'model': model, 'input': round2_input, **settings}
            status2, body2 = do_request(
                session,
                base_url=base_url,
                headers=headers,
                path='/responses',
                payload=payload2,
                timeout=timeout,
            )
            summary2 = summarize_body(surface, body2)
        else:
            system, round1_user, _ = build_cache_messages(nonce_prefix)
            payload1 = {
                'model': model,
                'messages': [
                    {'role': 'system', 'content': system},
                    {'role': 'user', 'content': round1_user},
                ],
            }
            status1, body1 = do_request(
                session,
                base_url=base_url,
                headers=headers,
                path='/chat/completions',
                payload=payload1,
                timeout=timeout,
            )
            summary1 = summarize_body(surface, body1)
            assistant_text = summary1.get('text')
            payload2 = {
                'model': model,
                'messages': [
                    {'role': 'system', 'content': system},
                    {'role': 'user', 'content': round1_user},
                    {'role': 'assistant', 'content': assistant_text},
                    {'role': 'user', 'content': 'Round2 task: given the exact prior exchange, reply with token R2 only.'},
                ],
            }
            status2, body2 = do_request(
                session,
                base_url=base_url,
                headers=headers,
                path='/chat/completions',
                payload=payload2,
                timeout=timeout,
            )
            summary2 = summarize_body(surface, body2)

        attempt = {
            'surface': surface,
            'settings': settings,
            'round1_status': status1,
            'round1': summary1,
            'round2_status': status2,
            'round2': summary2,
        }
        attempts.append(attempt)

        if status1 == 200 and status2 == 200:
            return {
                'surface': surface,
                'nonce_prefix_sha_fragment': nonce_prefix[:48],
                'attempts': attempts,
                'selected_attempt': attempt,
                'second_round_cached_tokens': summary2.get('cached_tokens'),
            }

    return {
        'surface': surface,
        'nonce_prefix_sha_fragment': nonce_prefix[:48],
        'attempts': attempts,
        'selected_attempt': attempts[-1] if attempts else None,
        'second_round_cached_tokens': None,
    }


def main() -> None:
    args = parse_args()
    api_key = load_api_key(args)
    catalog = load_catalog(args.catalog)

    session = requests.Session()
    headers = {
        'Authorization': f'Bearer {api_key}',
        'Content-Type': 'application/json',
    }

    report: dict[str, Any] = {
        'tested_at': dt.datetime.utcnow().replace(microsecond=0).isoformat() + 'Z',
        'base_url': args.base_url.rstrip('/'),
        'catalog': str(args.catalog),
        'max_attempts': args.max_attempts,
        'timeout_seconds': args.timeout,
        'models': [],
    }

    extra_chat_models = set(args.extra_chat_models)

    for model in args.models:
        model_meta = catalog.get(model) or {}
        official_surfaces = model_meta.get('api_surfaces') or []
        surfaces = ordered_surfaces(model, official_surfaces, extra_chat_models)
        entry: dict[str, Any] = {
            'model': model,
            'official_surfaces': official_surfaces,
            'probed_surfaces': surfaces,
            'source_url': model_meta.get('source_url'),
        }

        availability = run_availability_probe(
            session,
            model=model,
            surfaces=surfaces,
            base_url=args.base_url.rstrip('/'),
            headers=headers,
            timeout=args.timeout,
            max_attempts=args.max_attempts,
        )
        entry['availability'] = availability
        preferred_surface = first_success_surface(availability)
        entry['preferred_surface'] = preferred_surface

        if not args.availability_only and args.include_tools and preferred_surface in {'responses', 'chat.completion'}:
            entry['tool_probe'] = run_tool_probe(
                session,
                model=model,
                surface=preferred_surface,
                base_url=args.base_url.rstrip('/'),
                headers=headers,
                timeout=args.timeout,
            )

        if not args.availability_only and args.include_cache and preferred_surface in {'responses', 'chat.completion'}:
            entry['cache_probe'] = run_cache_probe(
                session,
                model=model,
                surface=preferred_surface,
                base_url=args.base_url.rstrip('/'),
                headers=headers,
                timeout=args.timeout,
            )

        report['models'].append(entry)

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
    print(args.output)
    print(json.dumps(report, ensure_ascii=False, indent=2)[:12000])


if __name__ == '__main__':
    main()
