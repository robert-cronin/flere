#!/usr/bin/env ruby
# Manual hosted-VM validation only. Derived from the retained Homebrew lifecycle
# checks; never use this helper on a developer machine or self-hosted runner.
require 'json'
require 'digest'
require 'fileutils'
require 'open3'
require 'tmpdir'

module HomebrewMacCI
  PREFIX = '/opt/homebrew'.freeze
  BREW = (PREFIX + '/bin/brew').freeze
  NAMES = %w[flere flere-connect].freeze
  FORMULAS = NAMES.map { |n| 'robert-cronin/flere/' + n }.freeze
  TAP_FILES = %w[Formula/flere.rb Formula/flere-connect.rb LICENSE README.md formula.rb.in render.py verify.py].sort.freeze
  PROTECTED = %w[.local/state/flere .config/flere-connect .local/bin].freeze
  ABSENT = %w[.config/flere .local/state/flere-connect .cache/flere .cache/flere-connect .local/share/flere .local/share/flere-connect].freeze
  SOURCE_LIMIT = 64 * 1024 * 1024
  LOG_LIMIT = 32 * 1024 * 1024

  def self.need(value, message)
    raise message unless value
  end

  def self.host_guard(env, platform = RUBY_PLATFORM)
    expected = {'GITHUB_ACTIONS' => 'true', 'RUNNER_ENVIRONMENT' => 'github-hosted',
                'GITHUB_REPOSITORY' => 'robert-cronin/flere', 'GITHUB_REF' => 'refs/heads/main',
                'GITHUB_EVENT_NAME' => 'workflow_dispatch', 'RUNNER_OS' => 'macOS', 'RUNNER_ARCH' => 'ARM64'}
    need(expected.all? { |key, value| env[key] == value }, 'requires the manual main workflow on a GitHub-hosted macOS ARM64 VM')
    need(platform.include?('darwin') && platform.match?(/arm64|aarch64/), 'native arm64 Ruby required')
    need(%w[macos-15 macos-26].include?(env['VALIDATION_RUNNER']), 'unexpected runner label')
    need(env.fetch('VALIDATION_WORKFLOW_SHA', '').match?(/\A[0-9a-f]{40}\z/), 'full workflow SHA required')
    need(%w[GITHUB_RUN_ID GITHUB_RUN_ATTEMPT].all? { |key| env.fetch(key, '').match?(/\A[1-9][0-9]*\z/) }, 'run identity missing')
  end

  def self.child_environment(root)
    home = root + '/home'
    {'HOME' => home, 'PATH' => PREFIX + '/bin:' + PREFIX + '/sbin:/usr/bin:/bin:/usr/sbin:/sbin',
     'LANG' => 'en_US.UTF-8', 'LC_ALL' => 'en_US.UTF-8', 'TERM' => 'dumb', 'CI' => 'true',
     'TMPDIR' => root + '/temp', 'XDG_CONFIG_HOME' => home + '/.config', 'XDG_CACHE_HOME' => home + '/.cache',
     'XDG_DATA_HOME' => home + '/.local/share', 'XDG_STATE_HOME' => home + '/.local/state',
     'HOMEBREW_CACHE' => root + '/brew-cache', 'HOMEBREW_LOGS' => root + '/brew-logs',
     'HOMEBREW_TEMP' => root + '/temp', 'HOMEBREW_USER_CONFIG_HOME' => home + '/.config/homebrew',
     'HOMEBREW_NO_AUTO_UPDATE' => '1', 'HOMEBREW_NO_ANALYTICS' => '1', 'CARGO_HOME' => root + '/cargo-cache',
     'CARGO_BUILD_JOBS' => '3', 'GIT_CONFIG_NOSYSTEM' => '1', 'GIT_CONFIG_GLOBAL' => '/dev/null',
     'GIT_TERMINAL_PROMPT' => '0', 'PYTHONDONTWRITEBYTECODE' => '1'}
  end

  def self.regular(path, maximum)
    st = File.lstat(path)
    need(st.file? && st.size > 0 && st.size <= maximum, 'regular file size/type differs: ' + File.basename(path))
    File.binread(path, maximum + 1).tap { |bytes| need(bytes.bytesize == st.size, 'file changed while reading') }
  end

  def self.digest(path, maximum = SOURCE_LIMIT)
    Digest::SHA256.hexdigest(regular(path, maximum))
  end

  def self.save(name, value)
    File.write(@evidence + '/' + name, JSON.pretty_generate(value) + "\n")
  end

  def self.snapshot
    result = {}
    PROTECTED.each do |relative|
      root = @env.fetch('HOME') + '/' + relative
      ([root] + Dir.glob(root + '/**/*', File::FNM_DOTMATCH).reject { |p| %w[. ..].include?(File.basename(p)) }).sort.each do |path|
        st = File.lstat(path)
        need(st.file? || st.directory?, 'unexpected synthetic state file type')
        result[path.delete_prefix(@env.fetch('HOME') + '/')] = {
          'mode' => st.mode & 0o7777, 'type' => st.file? ? 'file' : 'directory',
          'sha256' => st.file? ? digest(path, 65536) : nil}
      end
    end
    result
  end

  def self.check_state
    return unless @baseline
    need(snapshot == @baseline, 'synthetic state or manual-install sentinel changed')
    need(ABSENT.none? { |p| File.exist?(@env['HOME'] + '/' + p) || File.symlink?(@env['HOME'] + '/' + p) }, 'application state appeared')
  end

  def self.stop_group(waiter)
    begin
      Process.kill('TERM', -waiter.pid)
    rescue Errno::ESRCH
      # Already exited.
    end
    waiter.join(2)
    # The direct child may exit while a descendant still holds its pipe. Kill
    # the owned process group even when that direct child is already reaped.
    begin
      Process.kill('KILL', -waiter.pid)
    rescue Errno::ESRCH
      # Already exited.
    end
    need(waiter.join(5), 'owned command failed to terminate')
  end

  def self.command(label, argv, timeout: 120, limit: 1024 * 1024)
    start = Process.clock_gettime(Process::CLOCK_MONOTONIC)
    timeout = [timeout, @deadline - start].min if @deadline
    outputs = ['', ''].map(&:b)
    status = nil
    error = nil
    puts JSON.generate({'step' => label, 'status' => 'started'})
    begin
      need(timeout > 0, 'overall lifecycle deadline exceeded')
      Open3.popen3(@env, *argv, chdir: @root, pgroup: true, unsetenv_others: true) do |stdin, stdout, stderr, waiter|
        stdin.close
        streams = {stdout => 0, stderr => 1}
        begin
          until streams.empty?
            need(Process.clock_gettime(Process::CLOCK_MONOTONIC) - start < timeout, label + ' timed out')
            ready = IO.select(streams.keys, nil, nil, 0.1)
            next unless ready
            ready[0].each do |io|
              bytes = io.read_nonblock(65536, exception: false)
              next if bytes == :wait_readable
              if bytes.nil?
                streams.delete(io)
              else
                outputs[streams.fetch(io)] << bytes
                @log_bytes = (@log_bytes || 0) + bytes.bytesize
                need(outputs.sum(&:bytesize) <= limit && @log_bytes <= LOG_LIMIT, label + ' output bound exceeded')
              end
            end
          end
          remaining = timeout - (Process.clock_gettime(Process::CLOCK_MONOTONIC) - start)
          need(remaining > 0 && waiter.join(remaining), label + ' exit timed out')
          status = waiter.value.exitstatus
          need(status == 0, label + ' failed')
        rescue Exception
          stop_group(waiter)
          raise
        end
      end
      check_state
    rescue StandardError => failure
      error = failure.message
    ensure
      # These logs contain only commands run with the allowlisted environment;
      # no GitHub token, ambient Cargo credentials or host HOME is inherited.
      %w[stdout stderr].each_with_index do |stream, i|
        File.binwrite(@evidence + '/' + label + '-' + stream + '.log', outputs[i])
      end
      step = {'name' => label, 'argv' => argv, 'exit_code' => status, 'error' => error,
              'seconds' => (Process.clock_gettime(Process::CLOCK_MONOTONIC) - start).round(3),
              'stdout_bytes' => outputs[0].bytesize, 'stderr_bytes' => outputs[1].bytesize,
              'stdout_sha256' => Digest::SHA256.hexdigest(outputs[0]), 'stderr_sha256' => Digest::SHA256.hexdigest(outputs[1])}
      @receipt['steps'] << step
      save('receipt.json', @receipt)
      # JSON escaping keeps terminal escapes and Actions workflow commands inert.
      puts JSON.generate({'step' => label, 'exit_code' => status, 'error' => error,
                          'failure_tail' => error ? outputs.join.byteslice([outputs.sum(&:bytesize) - 4096, 0].max, 4096).force_encoding('UTF-8').scrub : nil})
    end
    raise error if error
    outputs[0].force_encoding('UTF-8')
  end

  def self.git(label, *args)
    command(label, ['/usr/bin/git', '--no-replace-objects', '-c', 'core.hooksPath=/dev/null',
                    '-c', 'core.fsmonitor=false', *args], timeout: 180)
  end

  def self.validate_pins(pins)
    need(pins['schema_version'] == 1 && pins['tap_url'] == 'https://github.com/robert-cronin/homebrew-flere', 'unexpected pin schema/remote')
    need(pins.fetch('phases').keys.sort == %w[initial upgraded], 'unexpected phases')
    pins['phases'].each do |phase, pin|
      need(pin['tap_commit'].match?(/\A[0-9a-f]{40}\z/) && pin['source_sha256'].match?(/\A[0-9a-f]{64}\z/), 'malformed source/tap pin')
      version = phase == 'initial' ? '0.3.0' : '0.3.2'
      wanted = phase == 'initial' ? {'flere' => '0.3.0_1', 'flere-connect' => '0.3.0'} : NAMES.to_h { |n| [n, '0.3.2'] }
      need(pin['version'] == version && pin['kegs'] == wanted, 'release transition changed')
      need(pin['source_bytes'].is_a?(Integer) && pin['source_bytes'].between?(1, SOURCE_LIMIT), 'source bound differs')
      need(pin.fetch('files').keys.sort == TAP_FILES, 'tap allowlist differs')
    end
  end

  def self.verify_tap(pin, tap = @tap)
    files = Dir.glob(tap + '/**/*', File::FNM_DOTMATCH).reject do |p|
      %w[. ..].include?(File.basename(p)) || p == tap + '/.git' || p.start_with?(tap + '/.git/') || File.lstat(p).directory?
    end.map { |p| p.delete_prefix(tap + '/') }.sort
    need(files == TAP_FILES, 'unexpected tap files')
    pin.fetch('files').each do |name, meta|
      bytes = regular(tap + '/' + name, 65536)
      need(bytes.bytesize == meta['bytes'] && Digest::SHA256.hexdigest(bytes) == meta['sha256'], 'tap bytes differ: ' + name)
    end
    check_state
  end

  def self.select_tap(phase, pin)
    git(phase + '-tap-checkout', '-C', @tap, 'checkout', '--detach', pin['tap_commit'])
    need(git(phase + '-tap-commit', '-C', @tap, 'rev-parse', 'HEAD').strip == pin['tap_commit'], 'tap commit differs')
    verify_tap(pin)
    command(phase + '-tap-readall', [BREW, 'readall', '--aliases', 'robert-cronin/flere'])
    verify_tap(pin)
  end

  def self.retain_source(phase, pin)
    command(phase + '-source-fetch', [BREW, 'fetch', '--formula', '--build-from-source', FORMULAS.first], timeout: 300)
    cache_path = command(phase + '-source-cache', [BREW, '--cache', '--build-from-source', FORMULAS.first]).strip
    source = File.realpath(cache_path)
    need(source.start_with?(@env['HOMEBREW_CACHE'] + '/'), 'source cache escaped private cache')
    bytes = regular(source, SOURCE_LIMIT)
    need(bytes.bytesize == pin['source_bytes'] && Digest::SHA256.hexdigest(bytes) == pin['source_sha256'], 'source bytes/hash differ')
    retained = @root + '/flere-' + pin.fetch('version') + '-source.tar.gz'
    File.binwrite(retained, bytes)
    command(phase + '-source-verify', ['/opt/homebrew/bin/python3', @checkout + '/packaging/homebrew/verify.py', retained, '--tap', @tap])
    command(phase + '-source-render', ['/opt/homebrew/bin/python3', @checkout + '/packaging/homebrew/render.py', retained,
                                     '--sha256', pin['source_sha256'], '--output', @tap, '--check'])
    @receipt[phase + '_source'] = {'sha256' => digest(retained), 'bytes' => bytes.bytesize}
    retained
  end

  def self.installed_versions(inventory, pin, previous = {})
    NAMES.each_with_index do |name, i|
      found = inventory.fetch('formulae').select { |f| f['full_name'] == FORMULAS[i] }
      need(found.length == 1, 'missing or duplicated installed formula')
      f = found.first
      need(f.dig('versions', 'stable') == pin['version'] && f['revision'] == pin['kegs'][name].split('_', 2)[1].to_i, 'formula version/revision differs')
      versions = f.fetch('installed').map { |v| v.fetch('version') }
      need(versions.include?(pin['kegs'][name]) && versions.uniq == versions &&
           (versions - [pin['kegs'][name], previous[name]].compact).empty?, 'unexpected installed keg version')
    end
  end

  def self.build_identity(info, name, version, rust)
    need(info['component'] == name && info['package_version'] == version, 'component/version differs')
    need(info['target'] == 'aarch64-apple-darwin' && info['profile'] == 'release', 'target/profile differs')
    need(info.dig('compatibility', 'remote_protocol', 'current') == 'flere-remote-v6', 'protocol differs')
    need(info['identity_kind'] == 'cargo_generation_stamp' && info.fetch('build_id').match?(/\A[a-f0-9]+-[a-f0-9]+-[a-f0-9]+\z/), 'build generation differs')
    need(info.fetch('rustc').strip == rust.strip, 'build did not use the recorded Homebrew Rust')
  end

  def self.smoke(phase, pin, source, rust)
    entries = {}
    NAMES.each_with_index do |name, index|
      command(phase + '-test-' + name, [BREW, 'test', FORMULAS[index]])
      binary = PREFIX + '/bin/' + name
      keg = PREFIX + '/Cellar/' + name + '/' + pin['kegs'][name]
      expected = keg + '/bin/' + name
      need(File.symlink?(binary) && File.realpath(binary) == expected, 'selected executable is not the expected keg')
      need((File.stat(expected).mode & 0o111) != 0, 'missing executable mode')
      command(phase + '-' + name + '-help', [binary, '--help'], timeout: 20, limit: 65536)
      version = command(phase + '-' + name + '-version', [binary, '--version'], timeout: 20, limit: 65536)
      need(version.start_with?(name + ' ' + pin['version'] + ' ('), 'CLI version differs')
      info = JSON.parse(command(phase + '-' + name + '-build-info', [binary, '--build-info'], timeout: 20, limit: 65536))
      build_identity(info, name, pin['version'], rust)
      need(command(phase + '-' + name + '-architecture', ['/usr/bin/lipo', '-archs', expected]).strip == 'arm64', 'binary is not native arm64')
      command(phase + '-' + name + '-libraries', ['/usr/bin/otool', '-L', expected])
      command(phase + '-' + name + '-providers', [BREW, 'linkage', '--reverse', FORMULAS[index]])
      receipt = JSON.parse(regular(keg + '/INSTALL_RECEIPT.json', 1024 * 1024))
      need(receipt['poured_from_bottle'] == false && receipt['built_as_bottle'] == false, 'Flere was not built from source')
      save(phase + '-' + name + '-INSTALL_RECEIPT.json', receipt)
      licenses = {}
      ['LICENSE', 'src/assets/fonts/OFL.txt', 'src/assets/fonts/LICENSE-Nerd-Fonts'].each do |member|
        bytes = command(phase + '-' + name + '-license-' + File.basename(member),
                        ['/usr/bin/tar', '-xOzf', source, 'flere-' + pin['version'] + '/' + member], limit: 65536)
        installed = keg + '/share/' + name + '/licenses/' + File.basename(member)
        need(regular(installed, 65536) == bytes.b, 'installed license differs')
        licenses[File.basename(member)] = digest(installed, 65536)
      end
      entries[name] = {'sha256' => digest(expected), 'bytes' => File.size(expected), 'build_info' => info, 'licenses' => licenses}
    end
    need(command(phase + '-companion-path', [PREFIX + '/bin/flere', 'ssh', '--help'], timeout: 20, limit: 65536).include?('flere-connect'), 'companion PATH lookup failed')
    @receipt[phase] = entries
    command(phase + '-linkage-strict', [BREW, 'linkage', '--test', '--strict', *FORMULAS])
  end

  def self.normalize_rust(initial)
    need(initial.fetch('formulae').none? { |f| NAMES.include?(f['name']) }, 'Flere is already installed')
    # Only this guarded disposable VM may normalize a preinstalled direct Rust.
    # Normal dependency protection remains active; a refusal is a failed run.
    rust_before = initial['formulae'].select { |f| f['name'] == 'rust' }
    command('remove-image-rust', [BREW, 'uninstall', '--formula', 'rust']) unless rust_before.empty?
    baseline = command('dependency-baseline', [BREW, 'list', '--formula', '--versions'])
    need(baseline.lines.none? { |l| (NAMES + ['rust']).include?(l.split.first) }, 'direct Rust/Flere keg remains')
    @receipt['preinstalled_rust_removed_normally'] = !rust_before.empty?
    @receipt['preinstalled_formula_names_after_normalization'] = baseline.lines.map { |l| l.split.first }.sort
  end

  def self.run
    host_guard(ENV) # Before any directory creation or external command.
    File.umask(0o077)
    parent = File.realpath(ENV.fetch('HOME')) + '/.cache/flere/tmp'
    FileUtils.mkdir_p(parent)
    @root = Dir.mktmpdir('b', parent)
    @evidence = @root + '/evidence'
    @env = child_environment(@root)
    [@evidence, @env['HOME'], @env['TMPDIR'], @env['HOMEBREW_CACHE'], @env['HOMEBREW_LOGS'], @env['CARGO_HOME']].each { |p| FileUtils.mkdir_p(p) }
    need(Dir.children(@env['CARGO_HOME']).empty? && Dir.children(@env['HOMEBREW_CACHE']).empty?, 'private cache is not fresh')
    @checkout = File.realpath(File.join(__dir__, '..'))
    @deadline = Process.clock_gettime(Process::CLOCK_MONOTONIC) + 3300
    @receipt = {'schema_version' => 1, 'status' => 'running', 'steps' => [], 'runtime_sessions_started' => false,
                'runner' => ENV['VALIDATION_RUNNER'], 'workflow_sha' => ENV['VALIDATION_WORKFLOW_SHA'],
                'run_id' => ENV['GITHUB_RUN_ID'], 'run_attempt' => ENV['GITHUB_RUN_ATTEMPT'],
                'initial_private_caches_empty' => true,
                'image_os' => ENV['ImageOS'], 'image_version' => ENV['ImageVersion'],
                'limits' => ['GitHub-hosted default-prefix source-formula acceptance; no physical UI, signing/notarization or live sessions',
                            'Fresh direct Homebrew Rust and empty private caches; image preinstalled transitive dependencies are recorded',
                            'Scoped trust and verified Git checkout; automatic tap cloning is not tested']}
    File.open(ENV.fetch('GITHUB_OUTPUT'), 'a') { |f| f.puts('evidence=' + @evidence) }
    begin
      need(File.realpath(ENV.fetch('GITHUB_WORKSPACE')) == @checkout, 'checkout differs from workflow workspace')
      need(git('workflow-commit', '-C', @checkout, 'rev-parse', 'HEAD').strip == ENV['VALIDATION_WORKFLOW_SHA'], 'workflow checkout differs')
      need(git('workflow-clean', '-C', @checkout, 'status', '--porcelain').empty?, 'workflow checkout is dirty')
      need(command('machine', ['/usr/bin/uname', '-m']).strip == 'arm64', 'native machine differs')
      command('os-details', ['/usr/bin/sw_vers'])
      os = command('os-version', ['/usr/bin/sw_vers', '-productVersion']).strip
      need(os.split('.').first == ENV['VALIDATION_RUNNER'].delete_prefix('macos-'), 'OS does not match matrix label')
      command('xcode', ['/usr/bin/xcodebuild', '-version'])
      command('xcode-path', ['/usr/bin/xcode-select', '-p'])
      command('brew-config-before', [BREW, 'config'])
      need(command('prefix', [BREW, '--prefix']).strip == PREFIX && File.realpath(PREFIX) == PREFIX, 'default prefix required')
      need(command('cellar', [BREW, '--cellar']).strip == PREFIX + '/Cellar', 'default Cellar required')
      repository = command('brew-repository', [BREW, '--repository']).strip
      need(repository == PREFIX, 'default macOS Brew repository required')
      @tap = repository + '/Library/Taps/robert-cronin/homebrew-flere'
      need(!File.exist?(@tap) && !File.symlink?(@tap), 'tap already exists')
      initial = JSON.parse(command('image-inventory', [BREW, 'info', '--json=v2', '--installed'], limit: 4 * 1024 * 1024))
      save('image-inventory.json', initial)
      normalize_rust(initial)
      need(Dir.children(@env['CARGO_HOME']).empty?, 'Cargo cache was seeded before installation')
      PROTECTED.each { |p| FileUtils.mkdir_p(@env['HOME'] + '/' + p) }
      File.write(@env['HOME'] + '/.local/state/flere/sentinel', "synthetic saved state\n")
      File.write(@env['HOME'] + '/.config/flere-connect/connections.json', "{\"synthetic\":true,\"connections\":[]}\n")
      File.write(@env['HOME'] + '/.local/bin/flere', "manual sentinel; never executed\n")
      @baseline = snapshot
      @receipt['state_before'] = @baseline
      pins = JSON.parse(regular(File.join(__dir__, 'homebrew-macos-ci-pins.json'), 65536))
      validate_pins(pins)
      @receipt['pins'] = pins
      command('brew-update', [BREW, 'update'], timeout: 600, limit: 4 * 1024 * 1024)
      command('brew-config', [BREW, 'config'])
      command('brew-version', [BREW, '--version'])
      git('brew-commit', '-C', repository, 'rev-parse', 'HEAD')
      command('trust-formulas', [BREW, 'trust', '--formula', *FORMULAS])
      need(!File.exist?(@tap) && !File.symlink?(@tap), 'trust created a tap')
      FileUtils.mkdir_p(File.dirname(@tap))
      git('tap-clone', 'clone', '--no-checkout', '--no-tags', '--single-branch', '--branch=main', '--template=', '--', pins['tap_url'], @tap)
      %w[initial upgraded].each do |phase|
        pin = pins['phases'].fetch(phase)
        select_tap(phase, pin)
        source = retain_source(phase, pin)
        FORMULAS.each_with_index { |f, i| command(phase + '-dependencies-' + NAMES[i], [BREW, 'deps', '--include-build', '--tree', f]) }
        command(phase + '-rust-metadata', [BREW, 'info', '--json=v2', 'rust'], limit: 2 * 1024 * 1024)
        command(phase + '-install', [BREW, phase == 'initial' ? 'install' : 'upgrade', '--formula', *FORMULAS], timeout: 1500, limit: 8 * 1024 * 1024)
        inventory = JSON.parse(command(phase + '-inventory', [BREW, 'info', '--json=v2', '--installed'], limit: 4 * 1024 * 1024))
        save(phase + '-inventory.json', inventory)
        @receipt[phase + '_new_formula_names'] = inventory['formulae'].map { |f| f['name'] }.sort - @receipt['preinstalled_formula_names_after_normalization']
        installed_versions(inventory, pin, phase == 'initial' ? {} : pins['phases']['initial']['kegs'])
        rust_formula = inventory['formulae'].select { |f| f['name'] == 'rust' && f['tap'] == 'homebrew/core' }
        need(rust_formula.length == 1 && !rust_formula.first.fetch('installed').empty?, 'Homebrew Rust dependency receipt missing')
        rust = command(phase + '-rust', [PREFIX + '/opt/rust/bin/rustc', '-vV'])
        command(phase + '-cargo', [PREFIX + '/opt/rust/bin/cargo', '-V'])
        verify_tap(pin)
        smoke(phase, pin, source, rust)
      end
      NAMES.each do |name|
        before = @receipt['initial'][name]; after = @receipt['upgraded'][name]
        need(before['sha256'] != after['sha256'] && before['build_info']['build_id'] != after['build_info']['build_id'], 'upgrade retained old payload/generation')
      end
      FORMULAS.reverse.each do |formula|
        name = formula.split('/').last
        2.times do |i|
          rack = PREFIX + '/Cellar/' + name
          break unless File.directory?(rack) && !Dir.children(rack).empty?
          command('uninstall-' + name + '-' + i.to_s, [BREW, 'uninstall', '--formula', formula])
        end
        need(!File.directory?(PREFIX + '/Cellar/' + name) || Dir.children(PREFIX + '/Cellar/' + name).empty?, 'keg remains')
        %w[bin opt].each { |d| need(!File.exist?(PREFIX + '/' + d + '/' + name) && !File.symlink?(PREFIX + '/' + d + '/' + name), 'command link remains') }
      end
      command('inventory-final', [BREW, 'list', '--formula', '--versions'])
      verify_tap(pins['phases']['upgraded'])
      check_state
      @receipt['state_after'] = snapshot
      @receipt['components_removed'] = true
      @receipt['status'] = 'passed'
    rescue StandardError => error
      @receipt['status'] = 'failed'
      @receipt['error'] = error.message
    ensure
      save('receipt.json', @receipt)
      puts JSON.generate({'status' => @receipt['status'], 'error' => @receipt['error']})
    end
    @receipt['status'] == 'passed' ? 0 : 1
  end
end

if $PROGRAM_NAME == __FILE__
  begin
    exit HomebrewMacCI.run
  rescue StandardError => error
    warn JSON.generate({'status' => 'refused', 'error' => error.message})
    exit 1
  end
end
