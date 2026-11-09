import unittest
from types import SimpleNamespace
from strict_sequence import enforce_budgets, install

class Tokenizer:
    mask_token = '[MASK]'
    def encode(self, text):
        return text.split()

class StrictBudgets(unittest.TestCase):
    def check(self, state='', options=None, max_len=512, budget=192, instruction='choose'):
        q = {'t':'choice', 'ins':instruction}
        enforce_budgets(Tokenizer(), state, q, max_len, budget,
                        lambda _: ['one', 'two'] if options is None else options)
    def test_context_boundary_and_mask_neutralization(self):
        # heading=3, options=4, prefix=10, so a length-16 context permits 5 state tokens.
        self.check('a b c d e', max_len=16)
        with self.assertRaisesRegex(ValueError, 'state context limit'):
            self.check('a b c d e f', max_len=16)
        self.check('a b c d e [MASK]', max_len=16)
    def test_option_boundary(self):
        self.check(options=[' '.join(['a']*48), 'b'])
        with self.assertRaisesRegex(ValueError, 'option token limit'):
            self.check(options=[' '.join(['a']*49), 'b'])
    def test_heading_and_option_count_limits(self):
        with self.assertRaisesRegex(ValueError, 'heading token budget'):
            self.check(instruction=' '.join(['a']*190))
        for options in [['one'], ['a']*256]:
            with self.assertRaisesRegex(ValueError, '2 through 255'):
                self.check(options=options)
        with self.assertRaisesRegex(ValueError, 'option token budget'):
            self.check(options=[' '.join(['a']*30)]*7)
    def test_valid_requests_are_forwarded_unchanged(self):
        calls=[]
        def original(*args, **kwargs):
            calls.append((args,kwargs));return 'encoded'
        sequence=SimpleNamespace(build_sequence=original,render_options=lambda q:['one','two'])
        install(sequence)
        tok=Tokenizer();q={'t':'choice','ins':'choose'}
        self.assertEqual(sequence.build_sequence(tok,'state',q,512,192),'encoded')
        self.assertIs(calls[0][0][0],tok)
        self.assertIs(calls[0][0][2],q)
        with self.assertRaisesRegex(ValueError, 'state context limit'):
            sequence.build_sequence(tok,' '.join(['a']*600),q,512,192)
        self.assertEqual(len(calls),1)

if __name__ == '__main__':unittest.main()
