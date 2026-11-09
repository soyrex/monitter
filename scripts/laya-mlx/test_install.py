import json
import tempfile
import unittest
from pathlib import Path
import install

class InstallerTests(unittest.TestCase):
    def test_offline_install_and_refuse_overwrite(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp); python=root/'python'; python.write_text(''); python.chmod(0o755)
            serve=root/'serve'; (serve/'mlx_omarchy_laya').mkdir(parents=True); (serve/'mlx_omarchy_laya/server.py').touch()
            model=root/'model'; model.mkdir(); (model/'manifest.json').touch(); (model/'model.safetensors').touch()
            dest=root/'installed'; units=root/'units'
            unit=install.install(python,serve,model,dest,units)
            self.assertEqual(json.loads((dest/'runtime.json').read_text())['modelDir'],str(model))
            self.assertTrue((dest/'laya-cli').stat().st_mode & 0o100)
            self.assertIn(str(dest/'laya-cli'),unit.read_text())
            with self.assertRaises(ValueError): install.install(python,serve,model,dest,units)
    def test_preserves_venv_python_symlink(self):
        with tempfile.TemporaryDirectory() as temp:
            link=Path(temp)/'python'; link.symlink_to('/usr/bin/python3')
            self.assertEqual(install.checked_path(link), link)
    def test_late_failure_rolls_back_and_preserves_existing_files(self):
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp); python=root/'python'; python.touch(); python.chmod(0o755)
            serve=root/'serve'; (serve/'mlx_omarchy_laya').mkdir(parents=True); (serve/'mlx_omarchy_laya/server.py').touch()
            model=root/'model'; model.mkdir(); (model/'manifest.json').touch(); (model/'model.safetensors').touch()
            dest=root/'installed'; dest.mkdir(); (dest/'notes').write_text('keep')
            units=root/'units'; units.mkdir(); (units/'unrelated.service').write_text('keep')
            original=Path.open
            def failing(path, mode='r', *args, **kwargs):
                if path.name == 'monitter-laya-mlx.service' and mode == 'xb': raise OSError('injected failure')
                return original(path, mode, *args, **kwargs)
            with patch.object(Path,'open',failing):
                with self.assertRaises(OSError): install.install(python,serve,model,dest,units)
            self.assertEqual((dest/'notes').read_text(),'keep')
            self.assertEqual((units/'unrelated.service').read_text(),'keep')
            self.assertEqual(sorted(x.name for x in dest.iterdir()),['notes'])
            self.assertTrue(install.install(python,serve,model,dest,units).is_file())
    def test_rejects_unsafe_path(self):
        with self.assertRaises(ValueError): install.checked_path('/tmp/has space')

if __name__=='__main__': unittest.main()
