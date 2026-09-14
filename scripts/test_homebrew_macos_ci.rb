#!/usr/bin/env ruby
require 'minitest/autorun'
require 'rbconfig'
require_relative 'homebrew-macos-ci'

class HomebrewMacCITest < Minitest::Test
  H = HomebrewMacCI

  def setup
    parent = File.join(Dir.home, '.cache/flere/tmp')
    FileUtils.mkdir_p(parent)
    @root = Dir.mktmpdir('brew-ci-test-', parent)
    @env = H.child_environment(@root)
    @evidence = @root + '/evidence'
    [@evidence, @env['HOME'], @env['TMPDIR'], @env['HOMEBREW_CACHE']].each { |p| FileUtils.mkdir_p(p) }
    H.instance_variable_set(:@root, @root)
    H.instance_variable_set(:@evidence, @evidence)
    H.instance_variable_set(:@env, @env)
    H.instance_variable_set(:@baseline, nil)
    H.instance_variable_set(:@deadline, nil)
    H.instance_variable_set(:@log_bytes, 0)
    H.instance_variable_set(:@receipt, {'steps' => []})
    @pins = JSON.parse(File.read(File.join(__dir__, 'homebrew-macos-ci-pins.json')))
  end

  def teardown
    FileUtils.remove_entry(@root) if File.directory?(@root)
  end

  def ci_env
    {'GITHUB_ACTIONS' => 'true', 'RUNNER_ENVIRONMENT' => 'github-hosted',
     'GITHUB_REPOSITORY' => 'robert-cronin/flere', 'GITHUB_REF' => 'refs/heads/main',
     'GITHUB_EVENT_NAME' => 'workflow_dispatch', 'RUNNER_OS' => 'macOS', 'RUNNER_ARCH' => 'ARM64',
     'VALIDATION_RUNNER' => 'macos-15', 'VALIDATION_WORKFLOW_SHA' => 'a' * 40,
     'GITHUB_RUN_ID' => '123', 'GITHUB_RUN_ATTEMPT' => '1'}
  end

  def test_host_guard_accepts_only_fixed_manual_hosted_arm64_context
    H.host_guard(ci_env, 'arm64-darwin')
    {'RUNNER_ENVIRONMENT' => 'self-hosted', 'GITHUB_REF' => 'refs/heads/other',
     'GITHUB_EVENT_NAME' => 'push', 'GITHUB_REPOSITORY' => 'someone/fork',
     'RUNNER_ARCH' => 'X64', 'VALIDATION_RUNNER' => 'macos-latest',
     'GITHUB_ACTIONS' => 'false', 'VALIDATION_WORKFLOW_SHA' => 'main'}.each do |key, value|
      assert_raises(RuntimeError) { H.host_guard(ci_env.merge(key => value), 'arm64-darwin') }
    end
    assert_raises(RuntimeError) { H.host_guard(ci_env, 'x86_64-darwin') }
  end

  def test_real_entry_refuses_local_execution_before_creating_state
    out, err, status = Open3.capture3({'HOME' => @env['HOME'], 'PATH' => '/usr/bin:/bin'},
                                     RbConfig.ruby, File.join(__dir__, 'homebrew-macos-ci.rb'), unsetenv_others: true)
    refute status.success?
    assert_empty out
    assert_equal 'refused', JSON.parse(err)['status']
    assert_empty Dir.children(@env['HOME'])
  end

  def test_child_environment_excludes_tokens_caches_toolchain_and_sandbox_overrides
    assert_equal @root + '/cargo-cache', @env['CARGO_HOME']
    %w[GH_TOKEN GITHUB_TOKEN CARGO_REGISTRY_TOKEN RUSTUP_HOME RUSTUP_TOOLCHAIN CARGO_NET_OFFLINE HOMEBREW_NO_INSTALL_FROM_API HOMEBREW_NO_INSTALL_CLEANUP HOMEBREW_NO_SANDBOX GIT_DIR DYLD_LIBRARY_PATH].each { |key| refute @env.key?(key) }
    assert_equal '/dev/null', @env['GIT_CONFIG_GLOBAL']
  end

  def test_pins_cannot_silently_change_version_transition_or_allowlist
    H.validate_pins(@pins)
    @pins['phases']['upgraded']['version'] = '0.3.4'
    assert_raises(RuntimeError) { H.validate_pins(@pins) }
    @pins['phases']['upgraded']['version'] = '0.3.2'
    @pins['phases']['initial']['files']['private.log'] = {'bytes' => 1, 'sha256' => '0' * 64}
    assert_raises(RuntimeError) { H.validate_pins(@pins) }
  end

  def fixture_tap
    tap = @root + '/tap'
    files = H::TAP_FILES.to_h do |name|
      path = tap + '/' + name
      FileUtils.mkdir_p(File.dirname(path))
      bytes = 'public fixture ' + name
      File.binwrite(path, bytes)
      [name, {'bytes' => bytes.bytesize, 'sha256' => Digest::SHA256.hexdigest(bytes)}]
    end
    [tap, {'files' => files}]
  end

  def test_tap_integrity_rejects_changed_and_untracked_files
    tap, pin = fixture_tap
    H.verify_tap(pin, tap)
    File.write(tap + '/private.log', 'private fixture')
    assert_raises(RuntimeError) { H.verify_tap(pin, tap) }
    File.unlink(tap + '/private.log')
    File.write(tap + '/README.md', 'changed')
    assert_raises(RuntimeError) { H.verify_tap(pin, tap) }
  end

  def test_tap_integrity_rejects_directory_symlinks_and_linked_inputs
    tap, pin = fixture_tap
    File.symlink(@evidence, tap + '/extra-directory')
    assert_raises(RuntimeError) { H.verify_tap(pin, tap) }
    File.unlink(tap + '/extra-directory')
    File.rename(tap + '/LICENSE', @root + '/license')
    File.symlink(@root + '/license', tap + '/LICENSE')
    assert_raises(RuntimeError) { H.verify_tap(pin, tap) }
  end

  def test_source_copy_survives_cache_cleanup_and_checks_digest
    file = @env['HOMEBREW_CACHE'] + '/source.tar.gz'
    File.write(file, 'known archive fixture')
    pin = {'version' => '0.3.0', 'source_bytes' => File.size(file), 'source_sha256' => H.digest(file)}
    H.instance_variable_set(:@checkout, @root)
    H.instance_variable_set(:@tap, @root + '/tap')
    calls = []
    callback = proc { |label, *args| calls << [label, args]; label.end_with?('-source-cache') ? file + "\n" : '' }
    retained = H.stub(:command, callback) { H.retain_source('initial', pin) }
    File.unlink(file)
    assert_equal 'flere-0.3.0-source.tar.gz', File.basename(retained)
    assert_equal 'known archive fixture', File.read(retained)
    assert calls.any? { |c| c.first == 'initial-source-verify' }
    assert calls.any? { |c| c.first == 'initial-source-render' }
    File.write(file, 'corrupt archive data!')
    assert_raises(RuntimeError) { H.stub(:command, callback) { H.retain_source('upgraded', pin.merge('version' => '0.3.2')) } }
    refute File.exist?(@root + '/flere-0.3.2-source.tar.gz')
  end

  def test_source_rejects_cache_escape_and_size_bound
    file = @root + '/outside'
    File.write(file, 'archive')
    callback = proc { |label, *| label.end_with?('-source-cache') ? file : '' }
    assert_raises(RuntimeError) { H.stub(:command, callback) { H.retain_source('initial', {'version' => '0.3.0', 'source_bytes' => 7, 'source_sha256' => H.digest(file)}) } }
    assert_raises(RuntimeError) { H.regular(file, 6) }
  end

  def make_state
    H::PROTECTED.each { |p| FileUtils.mkdir_p(@env['HOME'] + '/' + p) }
    File.write(@env['HOME'] + '/.local/state/flere/sentinel', 'saved')
    H.instance_variable_set(:@baseline, H.snapshot)
  end

  def test_state_guard_detects_content_mode_and_new_application_state
    make_state
    H.check_state
    file = @env['HOME'] + '/.local/state/flere/sentinel'
    File.write(file, 'changed')
    assert_raises(RuntimeError) { H.check_state }
    File.write(file, 'saved')
    File.chmod(0o777, file)
    assert_raises(RuntimeError) { H.check_state }
    File.chmod(0o644, file)
    H.instance_variable_set(:@baseline, H.snapshot)
    FileUtils.mkdir_p(@env['HOME'] + '/.cache/flere')
    assert_raises(RuntimeError) { H.check_state }
  end

  def inventory(pin, old = {})
    {'formulae' => H::NAMES.each_with_index.map do |name, i|
      {'full_name' => H::FORMULAS[i], 'versions' => {'stable' => pin['version']},
       'revision' => pin['kegs'][name].split('_', 2)[1].to_i,
       'installed' => ([pin['kegs'][name], old[name]].compact.map { |v| {'version' => v} })}
    end}
  end

  def test_selected_version_checks_reject_old_duplicate_or_wrong_revision
    pin = @pins['phases']['upgraded']; old = @pins['phases']['initial']['kegs']
    H.installed_versions(inventory(pin, old), pin, old)
    data = inventory(pin, old); data['formulae'][0]['installed'].shift
    assert_raises(RuntimeError) { H.installed_versions(data, pin, old) }
    data = inventory(pin); data['formulae'][0]['revision'] = 1
    assert_raises(RuntimeError) { H.installed_versions(data, pin) }
    data = inventory(pin); data['formulae'][0]['installed'] *= 2
    assert_raises(RuntimeError) { H.installed_versions(data, pin) }
  end

  def test_build_is_exact_phase_native_release_and_homebrew_rust
    info = {'component' => 'flere', 'package_version' => '0.3.2', 'target' => 'aarch64-apple-darwin',
            'profile' => 'release', 'identity_kind' => 'cargo_generation_stamp', 'build_id' => 'a-b-c',
            'compatibility' => {'remote_protocol' => {'current' => 'flere-remote-v6'}}, 'rustc' => 'brew rust'}
    H.build_identity(info, 'flere', '0.3.2', 'brew rust')
    {'package_version' => '0.3.3', 'target' => 'x86_64-apple-darwin', 'profile' => 'debug',
     'identity_kind' => 'source', 'rustc' => 'ambient rustup'}.each do |key, value|
      assert_raises(RuntimeError) { H.build_identity(info.merge(key => value), 'flere', '0.3.2', 'brew rust') }
    end
  end

  def test_rust_normalization_uses_only_normal_uninstall_and_requires_absence
    calls = []
    callback = proc { |label, argv, *| calls << argv; label == 'dependency-baseline' ? "python@3.14 3.14.7\n" : '' }
    H.stub(:command, callback) { H.normalize_rust({'formulae' => [{'name' => 'rust'}]}) }
    assert_equal [H::BREW, 'uninstall', '--formula', 'rust'], calls.first
    assert H.instance_variable_get(:@receipt)['preinstalled_rust_removed_normally']
    refusal = proc { |*| raise 'normal uninstall dependency protection refused' }
    assert_raises(RuntimeError) { H.stub(:command, refusal) { H.normalize_rust({'formulae' => [{'name' => 'rust'}]}) } }
    remaining = proc { |label, *| label == 'dependency-baseline' ? "rust 1.98.1\n" : '' }
    assert_raises(RuntimeError) { H.stub(:command, remaining) { H.normalize_rust({'formulae' => []}) } }
  end

  def test_real_command_does_not_inherit_ambient_environment
    code = <<~'CHILD'
      require ARGV.shift
      require 'rbconfig'
      h = HomebrewMacCI
      root = ARGV.shift
      h.instance_variable_set(:@root, root)
      h.instance_variable_set(:@evidence, root + '/evidence')
      h.instance_variable_set(:@env, h.child_environment(root))
      h.instance_variable_set(:@receipt, {'steps' => []})
      h.command('environment', [RbConfig.ruby, '-e', 'print ENV.key?("FLERE_PRIVATE_TEST_MARKER")'])
    CHILD
    _, stderr, status = Open3.capture3({'FLERE_PRIVATE_TEST_MARKER' => 'synthetic-only'}, RbConfig.ruby,
                                      '-e', code, File.join(__dir__, 'homebrew-macos-ci.rb'), @root)
    assert status.success?, stderr
    assert_equal 'false', File.read(@evidence + '/environment-stdout.log')
  end

  def test_real_command_nonzero_keeps_short_failure_and_escapes_control_output
    out, = capture_io do
      assert_raises(RuntimeError) { H.command('failure', [RbConfig.ruby, '-e', "STDERR.write(\"::error::synthetic\\n\\e[31m\");exit 9"]) }
    end
    final = JSON.parse(out.lines.last)
    assert_equal 9, final['exit_code']
    assert_includes final['failure_tail'], '::error::synthetic'
    refute out.lines.any? { |line| line.start_with?('::') || line.include?("\e") }
    assert_includes File.read(@evidence + '/failure-stderr.log'), 'synthetic'
  end

  def test_real_command_deadline_and_output_bound
    capture_io do
      error = assert_raises(RuntimeError) { H.command('deadline', [RbConfig.ruby, '-e', 'sleep 30'], timeout: 0.2) }
      assert_includes error.message, 'timed out'
      error = assert_raises(RuntimeError) { H.command('flood', [RbConfig.ruby, '-e', "STDOUT.write('x'*100000);sleep 30"], limit: 512) }
      assert_includes error.message, 'output bound'
    end
    assert_operator File.size(@evidence + '/flood-stdout.log'), :<=, 512 + 65536
  end

  def test_completed_parent_with_inherited_pipe_cannot_escape_timeout_cleanup
    capture_io do
      assert_raises(RuntimeError) do
        H.command('inherited-pipe', [RbConfig.ruby, '-e', 'STDOUT.sync=true; fork { puts Process.pid; sleep 30 }; exit 0'], timeout: 0.3)
      end
    end
    pid = Integer(File.read(@evidence + '/inherited-pipe-stdout.log').strip)
    stopped = false
    20.times do
      begin
        Process.kill(0, pid)
      rescue Errno::ESRCH
        stopped = true
        break
      end
      sleep 0.02
    end
    assert stopped, 'owned pipe-holding descendant remains'
  end
end
