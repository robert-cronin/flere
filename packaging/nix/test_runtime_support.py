#!/usr/bin/env python3
"""Pure protocol/budget regressions; never execute Nix, QEMU or Flere."""
import copy
import importlib.util
import json
import os
from pathlib import Path
import re
from types import SimpleNamespace
import tempfile
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('smoke', HERE/'runtime-smoke.py')
smoke = importlib.util.module_from_spec(spec)
spec.loader.exec_module(smoke)
BUDGET = re.search(r"<<'PY_BUDGET'\n(.*?)\nPY_BUDGET\n",(HERE/'runtime-check.sh').read_text(),re.S).group(1)
EPOCH = 'd'*32
TAB = {'id':17,'run':'a'*32,'pid':812,'kind':'shell','path':'','alive':True}
SNAPSHOT = {'protocol':4,'epoch':EPOCH,'active_workspace':9,
            'workspaces':[{'id':9,'selected_tab':17,'tabs':[TAB]}]}
DRV = '/nix/store/'+'0'*32+'-vm-test-run-flere-tcg.drv'
FETCH = 'these 2 paths will be fetched (20.50 MiB download, 60.25 MiB unpacked):\n'


class Protocol(unittest.TestCase):
    def test_selected_tab_is_an_id_and_identity_keeps_run_pid(self):
        row = smoke.workspace(copy.deepcopy(SNAPSHOT),EPOCH)
        self.assertEqual(row['selected_tab'],17)
        self.assertEqual(smoke.identity(row['tabs'][0]), {k:TAB[k] for k in ('id','run','pid','kind','path')})

    def test_epoch_and_workspace_selection_changes_refuse(self):
        for key, value in [('epoch','b'*32),('active_workspace',10),('protocol',3)]:
            snapshot = copy.deepcopy(SNAPSHOT)
            snapshot[key] = value
            with self.subTest(key=key), self.assertRaises(RuntimeError):
                smoke.workspace(snapshot,EPOCH)

    def test_unexpected_native_tab_or_extra_workspace_refuses(self):
        snapshot = copy.deepcopy(SNAPSHOT)
        snapshot['workspaces'][0]['tabs'][0]['kind']='codex'
        with self.assertRaises(RuntimeError):
            smoke.workspace(snapshot,EPOCH)
        snapshot = copy.deepcopy(SNAPSHOT)
        snapshot['workspaces'].append(copy.deepcopy(snapshot['workspaces'][0]))
        with self.assertRaises(RuntimeError):
            smoke.workspace(snapshot,EPOCH)

    def test_malformed_or_stopped_session_cannot_authorize_input(self):
        for key, value in [('run',''),('run','a'*31),('pid',0),('id',0),('alive',False)]:
            tab = dict(TAB,**{key:value})
            with self.subTest(key=key,value=value), self.assertRaises(RuntimeError):
                smoke.identity(tab)


class Budget(unittest.TestCase):
    def evaluate(self, log, free, should_pass):
        cache = Path.home()/'.cache/flere/tmp'
        cache.mkdir(parents=True,exist_ok=True)
        with tempfile.TemporaryDirectory(prefix='budget-',dir=cache) as directory:
            p = Path(directory)
            (p/'complete-dry-run.log').write_text(log)
            with patch.dict(os.environ,{'FLERE_NIX_PROOF':directory}), \
                 patch('shutil.disk_usage',return_value=SimpleNamespace(free=free)):
                if should_pass:
                    exec(compile(BUDGET,'runtime-check.sh:PY_BUDGET','exec'),{})
                else:
                    with self.assertRaises(AssertionError):
                        exec(compile(BUDGET,'runtime-check.sh:PY_BUDGET','exec'),{})
            return json.loads((p/'budget.json').read_text()) if (p/'budget.json').exists() else None

    def test_complete_fetch_and_build_plan_preserves_reserve(self):
        row = self.evaluate('this derivation will be built:\n  '+DRV+'\n'+FETCH,30*1024**3,True)
        self.assertEqual(row['missing_unpacked_bytes'],int(60.25*1024**2))
        self.assertEqual(row['required_bytes'],row['missing_unpacked_bytes']+14*1024**3)
        self.assertEqual(row['free_reserve_bytes'],4*1024**3)

    def test_insufficient_space_refuses_before_realization(self):
        row = self.evaluate('  '+DRV+'\n'+FETCH,14*1024**3,False)
        self.assertFalse(row['passed'])

    def test_unplanned_heavy_build_refuses_fixed_scratch_budget(self):
        for name in ('linux-6.18','qemu-host-cpu-only-11.1','rustc-1.98.1','gcc-15.2','llvm-21.1'):
            log = '  '+DRV+'\n  /nix/store/'+'1'*32+'-'+name+'.drv\n'
            with self.subTest(name=name):
                row = self.evaluate(log,100*1024**3,False)
                self.assertEqual(len(row['unexpected_heavy_builds']),1)

    def test_unknown_fetch_units_and_incomplete_plan_refuse(self):
        self.evaluate('  '+DRV+'\nthese 2 paths will be fetched (2 TB download, 4 TB unpacked):\n',100*1024**3,False)
        self.evaluate('  /nix/store/'+'0'*32+'-flere-0.3.4.drv\n'+FETCH,100*1024**3,False)


if __name__ == '__main__':
    unittest.main()
