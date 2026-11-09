"""Keep the native Laya service's strict token budgets around MLX encoding.

No weights or tensor operations change. The upstream MLX encoder truncates;
Monitter's existing native service rejects these requests instead.
"""
import json


def enforce_budgets(tok, state, q, max_len, head_max_len, render_options):
    options = render_options(q)
    if not 2 <= len(options) <= 255:
        raise ValueError('Questions require 2 through 255 options')
    mask = tok.mask_token
    def encode(text):
        return tok.encode(text.replace(mask, ' '))
    heading = encode('%s question: %s' % (q['t'], q['ins']))
    lengths = []
    for option in options:
        length = len(encode(' ' + option))
        if length > 48:
            raise ValueError('Question exceeds option token limit (48)')
        lengths.append(length + 1)  # each option starts with one mask marker
    used = sum(lengths)
    remaining = head_max_len - used
    if remaining < 16:
        per = max(4, (head_max_len - 16) // len(options))
        if any(length > per for length in lengths):
            raise ValueError('Question exceeds option token budget (%d tokens per option)' % per)
    heading_limit = max(8, remaining)
    if len(heading) > heading_limit:
        raise ValueError('Question exceeds heading token budget (%d tokens)' % heading_limit)
    if used + len(heading) > head_max_len:
        raise ValueError('Question exceeds question head token budget (%d tokens)' % head_max_len)
    # CLS + heading + SEP + marked options + SEP, followed by state + SEP.
    prefix_len = 1 + len(heading) + 1 + used + 1
    room = max(0, max_len - prefix_len - 1)
    state_text = state if isinstance(state, str) else json.dumps(state, ensure_ascii=False)
    if len(encode(state_text)) > room:
        raise ValueError('Question exceeds state context limit (%d tokens)' % room)
    if prefix_len + 1 > max_len:
        raise ValueError('Question exceeds final sequence limit (%d tokens)' % max_len)


def install(sequence):
    original = sequence.build_sequence
    def strict(tok, state, q, max_len, head_max_len, option_order=None, truncate_left=False):
        enforce_budgets(tok, state, q, max_len, head_max_len, sequence.render_options)
        return original(tok, state, q, max_len, head_max_len,
                        option_order=option_order, truncate_left=truncate_left)
    sequence.build_sequence = strict
