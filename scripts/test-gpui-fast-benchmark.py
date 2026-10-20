#!/usr/bin/env python3
"""Exercise fail-fast scheduling without starting a process or fixture."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec=importlib.util.spec_from_file_location('benchmark',Path(__file__).with_name('gpui-fast-benchmark.py'))
b=importlib.util.module_from_spec(spec)
spec.loader.exec_module(b)


class BenchmarkTests(unittest.TestCase):
    def test_invalid_diagnostic_stops_after_preserved_first_receipt(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            for side in ('reference','treatment'):
                (root/side).mkdir()
                (root/side/'machine-conditions.json').write_text('{}')
            launches=[]
            def launch(command):
                launches.append(command)
                output=Path(command[command.index('--output')+1]);output.mkdir()
                (output/'metrics.json').write_text(json.dumps(dict(measuredSeconds=60,drawMs=dict(count=0))))
                class Finished:
                    returncode=0
                    def poll(self):return 0
                return Finished()
            argv=['benchmark','--reference','/unused/ref','--treatment','/unused/treatment','--output',str(root),'--repetitions','5']
            with patch.object(b.sys,'argv',argv), patch.object(b,'compiler_processes',return_value=[]), patch.object(b.subprocess,'Popen',side_effect=launch), patch.object(b.time,'sleep'), patch('builtins.print'):
                with self.assertRaisesRegex(SystemExit,'window/workload diagnostic rejected'):
                    b.main()
            self.assertEqual(1,len(launches))
            receipts=json.loads((root/'benchmark-receipts.json').read_text())
            self.assertEqual(1,len(receipts))
            self.assertFalse(receipts[0]['windowConditionsEligible'])
            self.assertTrue(Path(receipts[0]['output'],'metrics.json').exists())
            self.assertTrue(Path(receipts[0]['output'],'environment-guard.json').exists())


if __name__=='__main__':unittest.main()
