"""Regression tests for the relocation guardrail in ../wscript.

The guardrail is the safety net for a bug that once shipped a broken app: if
`-C relocation-model=pic` fails to reach rustc, LLVM bakes absolute addresses
into Thumb instructions, and the Pebble loader -- which only fixes up
.rel.data and .got -- never corrects them, so every &'static pointer is
garbage at runtime.

The parser has been wrong twice, so it gets standing coverage. Run with:

    python3 -m unittest discover -s examples/hello/tests

The wscript is not importable as a module (waf executes it), so these tests
exec its module-level definitions in an isolated namespace and stub out both
`subprocess` and `waflib`.
"""

import os
import shutil
import subprocess as real_subprocess
import sys
import tempfile
import types
import unittest

WSCRIPT = os.path.join(os.path.dirname(__file__), os.pardir, 'wscript')


class _WafError(Exception):
    """Stand-in for waflib.Errors.WafError."""


def _install_fake_waflib():
    waflib = types.ModuleType('waflib')
    errors = types.ModuleType('waflib.Errors')
    errors.WafError = _WafError
    waflib.Errors = errors
    sys.modules['waflib'] = waflib
    sys.modules['waflib.Errors'] = errors


def _load_wscript():
    _install_fake_waflib()
    ns = {'__name__': 'wscript_under_test'}
    with open(WSCRIPT) as f:
        exec(compile(f.read(), WSCRIPT, 'exec'), ns)
    return ns


def _header(section, entries=1):
    return ("Relocation section '%s' at offset 0x1234 contains %d entries:"
            % (section, entries))


def _entry(reloc_type, symbol='some_symbol', offset='0000016c'):
    # Mirrors readelf's layout: type is rendered in a %-17.17s field.
    return '%s  00001402 %-17.17s 000000b0   %s' % (offset, reloc_type, symbol)


class RelocationCheckTest(unittest.TestCase):
    def setUp(self):
        self.ns = _load_wscript()

    def check(self, readelf_output, readelf='readelf'):
        """Run _check_relocations against canned readelf output.

        Returns the raised WafError, or None if the check passed.
        """
        self.ns['subprocess'] = types.SimpleNamespace(
            check_output=lambda *a, **k: readelf_output.encode(),
            CalledProcessError=real_subprocess.CalledProcessError,
        )
        try:
            self.ns['_check_relocations']('fake.elf', readelf)
            return None
        except _WafError as e:
            return e

    def assertClean(self, output, msg=''):
        err = self.check(output)
        self.assertIsNone(err, '%s\nunexpectedly flagged:\n%s' % (msg, err))

    def assertFlagged(self, output, msg=''):
        self.assertIsNotNone(self.check(output), '%s\nwas not flagged' % msg)

    # --- the good case -------------------------------------------------

    def test_pc_relative_text_is_clean(self):
        self.assertClean('\n'.join([
            _header('.rel.text', 3),
            _entry('R_ARM_THM_CALL'),
            _entry('R_ARM_REL32'),
            _entry('R_ARM_THM_JUMP24'),
        ]), 'a correct PIC build must not be flagged')

    def test_empty_output_is_clean(self):
        self.assertClean('', 'no relocation sections at all')

    # --- detection of the failure mode this guard exists for -----------

    def test_abs32_in_text_is_flagged(self):
        self.assertFlagged('\n'.join([
            _header('.rel.text'), _entry('R_ARM_ABS32', '.text'),
        ]), 'R_ARM_ABS32 in .rel.text')

    def test_thumb_movw_movt_abs_are_flagged(self):
        # readelf truncates these to 17 chars; the guard must match that form.
        for t in ('R_ARM_THM_MOVW_AB', 'R_ARM_THM_MOVT_AB'):
            self.assertFlagged('\n'.join([
                _header('.rel.text'), _entry(t, '.Lanon.1234'),
            ]), t)

    # --- I1: symbol names must not be mistaken for relocation types ----

    def test_underscore_AB_in_symbol_name_is_not_flagged(self):
        # Rust v0 mangling can put '_AB' in a symbol; a substring test would
        # trip on this benign PC-relative call.
        self.assertClean('\n'.join([
            _header('.rel.text'),
            _entry('R_ARM_THM_CALL', '_RNvCs_ABxyz_4core3fmt5write'),
        ]), "'_AB' inside a symbol name")

    def test_abs32_text_in_symbol_name_is_not_flagged(self):
        self.assertClean('\n'.join([
            _header('.rel.text'),
            _entry('R_ARM_THM_CALL', 'handler_for_R_ARM_ABS32'),
        ]), 'a relocation-type name appearing inside a symbol')

    # --- I4: section coverage ------------------------------------------

    def test_per_function_text_sections_are_checked(self):
        self.assertFlagged('\n'.join([
            _header('.rel.text.main'), _entry('R_ARM_ABS32', '.text'),
        ]), '.rel.text.<fn>')

    def test_rodata_is_checked(self):
        self.assertFlagged('\n'.join([
            _header('.rel.rodata.cst4'), _entry('R_ARM_ABS32', '.rodata'),
        ]), '.rel.rodata')

    # --- exempt sections ------------------------------------------------

    def test_rel_data_is_exempt(self):
        # inject_metadata.py emits load-time fixups for .rel.data*.
        self.assertClean('\n'.join([
            _header('.rel.data'), _entry('R_ARM_ABS32', '.data'),
        ]), '.rel.data is relocated by the SDK')

    def test_rel_data_rel_ro_is_exempt(self):
        # inject_metadata.py prefix-matches "'.rel.data", so this is covered.
        self.assertClean('\n'.join([
            _header('.rel.data.rel.ro'), _entry('R_ARM_ABS32', '.data.rel.ro'),
        ]), '.rel.data.rel.ro is relocated by the SDK')

    def test_rel_got_is_exempt(self):
        # The SDK relocates the whole .got range. The section readelf prints
        # is '.rel.got' -- an exemption spelled '.got' would never match.
        self.assertClean('\n'.join([
            _header('.rel.got'), _entry('R_ARM_ABS32', '.got'),
        ]), '.rel.got is relocated by the SDK')

    def test_debug_sections_are_exempt(self):
        # Real builds carry ~1100 ABS32 entries here; they are never loaded.
        self.assertClean('\n'.join([
            _header('.rel.debug_info', 2),
            _entry('R_ARM_ABS32', '.text'),
            _entry('R_ARM_ABS32', '.text'),
        ]), 'non-allocated debug relocations are inert')

    def test_exidx_prel31_is_clean(self):
        # .ARM.exidx IS allocated, so it is checked like any loaded section --
        # its legitimate PREL31 entries pass on type, not on section name.
        self.assertClean('\n'.join([
            _header('.rel.ARM.exidx.text.main'), _entry('R_ARM_PREL31', '.text'),
        ]), 'PREL31 is self-relative')

    def test_abs32_in_exidx_is_flagged(self):
        # .ARM.exidx carries AL in readelf -S and inject_metadata.py does not
        # fix it up, so a non-relative entry here would ship uncorrected.
        self.assertFlagged('\n'.join([
            _header('.rel.ARM.exidx.text.main'), _entry('R_ARM_ABS32', '.text'),
        ]), 'absolute relocation in a loaded exidx section')

    # --- parser state machine -------------------------------------------

    def test_section_state_resets_between_sections(self):
        self.assertFlagged('\n'.join([
            _header('.rel.text'), _entry('R_ARM_ABS32', '.text'),
            '',
            _header('.rel.debug_info'), _entry('R_ARM_ABS32', '.text'),
        ]), 'a bad .rel.text followed by an exempt section')

    def test_exempt_section_does_not_mask_a_later_bad_one(self):
        self.assertFlagged('\n'.join([
            _header('.rel.debug_info'), _entry('R_ARM_ABS32', '.text'),
            '',
            _header('.rel.text'), _entry('R_ARM_ABS32', '.text'),
        ]), 'an exempt section preceding a bad one')

    def test_bad_section_buried_late_in_large_output(self):
        # Regression: an earlier parser indexed a LINE LIST with a CHARACTER
        # offset, so sections far into the output were silently skipped.
        lines = [_header('.rel.ARM.exidx.text.' + 'a' * 64),
                 _entry('R_ARM_PREL31', '.text')]
        for i in range(200):
            lines.append(_header('.rel.debug_info'))
            lines.append(_entry('R_ARM_ABS32', '.text' + 'b' * 64))
        lines.append(_header('.rel.text'))
        lines.append(_entry('R_ARM_ABS32', '.text'))
        self.assertFlagged('\n'.join(lines),
                           'a bad section far into a large output')

    # --- fail-closed behaviour ------------------------------------------

    def test_unknown_relocation_type_is_flagged(self):
        # R_ARM_TARGET1 resolves to ABS32 on bare metal; a deny-list would
        # let it through. The guard must fail closed on anything unlisted.
        self.assertFlagged('\n'.join([
            _header('.rel.init_array'), _entry('R_ARM_TARGET1', 'some_ctor'),
        ]), 'unknown relocation type')

    def test_non_thumb_movw_abs_is_flagged(self):
        self.assertFlagged('\n'.join([
            _header('.rel.text'), _entry('R_ARM_MOVW_ABS_NC', '.Lanon.1'),
        ]), 'ARM (non-Thumb) absolute movw')

    def test_unrecognized_relocation_number_is_flagged(self):
        # readelf prints "unrecognized: f8" for a reloc number it cannot name.
        # That is the least-understood input there is, so it must fail closed;
        # an earlier version gated on startswith('R_') and skipped it.
        self.assertFlagged(
            '\n'.join([
                _header('.rel.text'),
                '000000e0  00009bf8 unrecognized: f8      00000e21   app_log',
            ]),
            'a relocation type readelf cannot name')

    def test_unparseable_section_header_fails_closed(self):
        self.assertFlagged('\n'.join([
            'Relocation section MALFORMED at offset 0x1 contains 1 entries:',
            _entry('R_ARM_ABS32', '.text'),
        ]), 'a header with no quoted section name')

    # --- the check must never silently succeed --------------------------

    def test_missing_readelf_raises_waferror_not_oserror(self):
        # A raw OSError would escape the caller's WafError handler and leave
        # an unverified .pbw installable.
        def boom(*a, **k):
            raise OSError(2, 'No such file or directory')

        self.ns['subprocess'] = types.SimpleNamespace(
            check_output=boom,
            CalledProcessError=real_subprocess.CalledProcessError,
        )
        with self.assertRaises(_WafError):
            self.ns['_check_relocations']('fake.elf', 'no-such-readelf')

    def test_readelf_nonzero_exit_raises_waferror(self):
        def boom(*a, **k):
            raise real_subprocess.CalledProcessError(1, 'readelf')

        self.ns['subprocess'] = types.SimpleNamespace(
            check_output=boom,
            CalledProcessError=real_subprocess.CalledProcessError,
        )
        with self.assertRaises(_WafError):
            self.ns['_check_relocations']('fake.elf', 'readelf')


class RelocationCheckMessageTest(unittest.TestCase):
    """The diagnostic has to point at the actual cause."""

    def setUp(self):
        self.ns = _load_wscript()

    def test_message_names_the_offenders_and_the_flags(self):
        self.ns['subprocess'] = types.SimpleNamespace(
            check_output=lambda *a, **k: '\n'.join([
                _header('.rel.text'), _entry('R_ARM_ABS32', '.text'),
            ]).encode(),
            CalledProcessError=real_subprocess.CalledProcessError,
        )
        with self.assertRaises(_WafError) as cm:
            self.ns['_check_relocations']('app.elf', 'readelf')
        msg = str(cm.exception)
        self.assertIn('app.elf', msg)
        self.assertIn('relocation-model=pic', msg)
        self.assertIn('.rel.text', msg)


class DiscardArtifactsTest(unittest.TestCase):
    """An unverified binary must never remain installable.

    Covers the second half of the guardrail: whatever goes wrong, the .pbw
    and per-platform .bin are removed before the error propagates.
    """

    def setUp(self):
        self.ns = _load_wscript()
        self.tmp = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.binaries = [{'platform': 'emery',
                          'app_elf': 'emery/pebble-app.elf'}]
        os.makedirs(os.path.join(self.tmp, 'emery'))
        self.pbw = os.path.join(self.tmp, 'hello.pbw')
        self.bin = os.path.join(self.tmp, 'emery', 'pebble-app.bin')
        self.elf = os.path.join(self.tmp, 'emery', 'pebble-app.elf')
        for p in (self.pbw, self.bin, self.elf):
            with open(p, 'wb') as f:
                f.write(b'x')

    def _node(self, _rel):
        return types.SimpleNamespace(abspath=lambda: self.elf)

    def _stub_readelf(self, behaviour):
        self.ns['subprocess'] = types.SimpleNamespace(
            check_output=behaviour,
            CalledProcessError=real_subprocess.CalledProcessError,
        )

    def assertArtifactsGone(self):
        self.assertFalse(os.path.exists(self.pbw), '.pbw survived')
        self.assertFalse(os.path.exists(self.bin), '.bin survived')

    def test_artifacts_survive_a_clean_verification(self):
        self._stub_readelf(lambda *a, **k: '\n'.join([
            _header('.rel.text'), _entry('R_ARM_THM_CALL'),
        ]).encode())
        self.ns['_verify_binaries'](self.tmp, self.binaries, self._node)
        self.assertTrue(os.path.exists(self.pbw), '.pbw wrongly removed')
        self.assertTrue(os.path.exists(self.bin), '.bin wrongly removed')

    def test_bad_relocation_discards_artifacts(self):
        self._stub_readelf(lambda *a, **k: '\n'.join([
            _header('.rel.text'), _entry('R_ARM_ABS32', '.text'),
        ]).encode())
        with self.assertRaises(_WafError):
            self.ns['_verify_binaries'](self.tmp, self.binaries, self._node)
        self.assertArtifactsGone()

    def test_missing_readelf_discards_artifacts(self):
        def boom(*a, **k):
            raise OSError(2, 'No such file or directory')

        self._stub_readelf(boom)
        with self.assertRaises(_WafError):
            self.ns['_verify_binaries'](self.tmp, self.binaries, self._node)
        self.assertArtifactsGone()

    def test_missing_elf_node_discards_artifacts(self):
        self._stub_readelf(lambda *a, **k: b'')
        with self.assertRaises(_WafError):
            self.ns['_verify_binaries'](self.tmp, self.binaries,
                                        lambda _rel: None)
        self.assertArtifactsGone()

    def test_unexpected_exception_still_discards_artifacts(self):
        # The handler is deliberately broad: any failure to COMPLETE the
        # check leaves the binary unverified.
        def boom(*a, **k):
            raise RuntimeError('something nobody anticipated')

        self._stub_readelf(boom)
        with self.assertRaises(RuntimeError):
            self.ns['_verify_binaries'](self.tmp, self.binaries, self._node)
        self.assertArtifactsGone()


if __name__ == '__main__':
    unittest.main()
