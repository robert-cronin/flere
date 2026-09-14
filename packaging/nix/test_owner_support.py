#!/usr/bin/env python3
"""Pure upgrade/profile and complete-closure gate checks; never run Nix or Flere."""
import copy
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('owner', Path(__file__).with_name('owner-probe.py'))
owner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(owner)
GIB = 1024**3
OLD = {'flere':'/nix/store/'+'a'*32+'-flere-0.3.3',
       'flere-connect':'/nix/store/'+'b'*32+'-flere-connect-0.3.3'}
NEW = {k:v.replace('0.3.3','0.3.4') for k,v in OLD.items()}
BEFORE = {'generation':'profile-1-link','environment':'/nix/store/'+'c'*32+'-user-environment',
          'packages':{k+'-0.3.3':v for k,v in OLD.items()}}
AFTER = {'generation':'profile-2-link','environment':'/nix/store/'+'d'*32+'-user-environment',
         'packages':{k+'-0.3.4':v for k,v in NEW.items()}}
DRVS = [path+'.drv' for path in [*OLD.values(),*NEW.values()]]
BUILD = 'these 4 derivations will be built:\n'+''.join('  '+p+'\n' for p in DRVS)
FETCH = 'this path will be fetched (1.00 MiB download, 2.25 MiB unpacked):\n  /nix/store/'+'e'*32+'-dependency\n'


class ProfileUpgrade(unittest.TestCase):
    def test_both_exact_versions_and_real_generation_change(self):
        for version, outputs in [('0.3.3', OLD), ('0.3.4', NEW)]:
            text=''.join(k+'-'+version+' '+v+'\n' for k,v in outputs.items())
            self.assertEqual(owner.profile_inventory(text,version,outputs), {k+'-'+version:v for k,v in outputs.items()})
        owner.require_upgrade(BEFORE,AFTER)

    def test_successful_noop_or_unchanged_environment_is_not_upgrade(self):
        for key in ('generation','environment','packages'):
            after=copy.deepcopy(AFTER)
            after[key]=BEFORE[key]
            with self.subTest(key=key), self.assertRaises(RuntimeError):
                owner.require_upgrade(BEFORE,after)

    def test_wrong_version_or_selected_payload_is_not_accepted(self):
        correct=''.join(k+'-0.3.4 '+v+'\n' for k,v in NEW.items())
        for text in [correct.replace('flere-0.3.4 ','flere-0.3.3 '),
                     correct.replace(NEW['flere'],OLD['flere']), correct.replace('0.3.4','0.3.5')]:
            with self.subTest(text=text), self.assertRaises(RuntimeError):
                owner.profile_inventory(text,'0.3.4',NEW)

    def test_missing_duplicate_or_extra_profile_entry_refuses(self):
        rows=[k+'-0.3.4 '+v+'\n' for k,v in NEW.items()]
        for text in [rows[0],rows[0]*2,''.join(rows)+rows[0],''.join(rows)+'unrelated /nix/store/extra\n']:
            with self.subTest(text=text), self.assertRaises(RuntimeError):
                owner.profile_inventory(text,'0.3.4',NEW)


class CompleteBudget(unittest.TestCase):
    def test_four_derivations_complete_fetch_and_exact_reserve(self):
        row=owner.build_budget(BUILD+FETCH,30*GIB)
        self.assertTrue(row['passed'])
        self.assertEqual(row['local_derivations'],DRVS)
        self.assertEqual(row['missing_unpacked_bytes'],2359296)
        self.assertEqual(row['required_bytes'],2359296+12*GIB)
        self.assertEqual(row['free_reserve_bytes'],4*GIB)

    def test_cached_closure_and_fractional_byte_rounding(self):
        self.assertTrue(owner.build_budget('',12*GIB)['passed'])
        row=owner.build_budget(FETCH.replace('2.25 MiB','1.1 B'),13*GIB)
        self.assertEqual(row['missing_unpacked_bytes'],2)

    def test_space_or_uncached_toolchain_refuses(self):
        self.assertFalse(owner.build_budget(BUILD+FETCH,12*GIB)['passed'])
        heavy='this derivation will be built:\n  /nix/store/'+'f'*32+'-rustc-1.98.1.drv\n'
        self.assertFalse(owner.build_budget(heavy,100*GIB)['passed'])

    def test_malformed_or_ambiguous_complete_budget_refuses(self):
        cases=[BUILD.replace('these 4','these 5'),BUILD.replace(DRVS[1],DRVS[0]),
               BUILD+BUILD,FETCH.replace('MiB unpacked','TB unpacked'),
               FETCH.replace('2.25','NaN'),FETCH.replace('2.25','-1'),
               FETCH.replace('this path','these 2 paths'),BUILD+'unknown plan line\n',
               BUILD.replace('.drv\n','\n',1)]
        for log in cases:
            with self.subTest(log=log), self.assertRaises(RuntimeError):
                owner.build_budget(log,100*GIB)
        with self.assertRaises(RuntimeError):
            owner.build_budget(BUILD,-1)


if __name__ == '__main__':
    unittest.main()
