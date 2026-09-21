import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const source = fileURLToPath(new URL('../test.ps1', import.meta.url));
const cases = [
  ['missing-deps', 'sdks', false],
  ['missing-python', 'sdks', false],
  ['alias-python', 'sdks', false],
  ['build-failure', 'sdks', false],
  ['test-failure', 'sdks', false],
  ['python-failure', 'sdks', false],
  ['throw', 'sdks', false],
  ['missing-command', 'sdks', false],
  ['native-failure', 'core', false],
  ['native-warning', 'core', true],
  ['aggregate-native-failure', 'all', false],
  ['success', 'sdks', true],
];

test('selected suites fail closed and preserve command failures', {
  skip: process.platform !== 'win32' ? 'Windows PowerShell regression' : false,
  timeout: 180_000,
}, async t => {
  const root = mkdtempSync(path.join(tmpdir(), 'worldos-selected-suites-'));
  t.diagnostic(`Retained fixture: ${root}`);
  const bytes = readFileSync(source);
  const hash = b => createHash('sha256').update(b).digest('hex');
  const shell = path.join(process.env.SystemRoot || 'C:/Windows', 'System32/WindowsPowerShell/v1.0/powershell.exe');
  for (const [scenario, suite, success] of cases) {
    await t.test(scenario, () => {
      const dir = path.join(root, scenario);
      mkdirSync(path.join(dir, 'scripts'), { recursive: true });
      writeFileSync(path.join(dir, 'scripts/test.ps1'), bytes);
      if (scenario !== 'missing-deps') mkdirSync(path.join(dir, 'node_modules'));
      const wrapper = String.raw`
param([string]$Scenario, [string]$Suite)
$global:LASTEXITCODE = 0
function global:Get-Command {
    param([string]$Name, [switch]$All, [string]$ErrorAction)
    if ($Name -ne 'python') { throw 'Unexpected discovery' }
    if ($Scenario -eq 'alias-python') { [pscustomobject]@{ Source = 'C:\Fixture\WindowsApps\python.exe' }; return }
    if ($Scenario -ne 'missing-python') { [pscustomobject]@{ Source = 'Invoke-FixturePython' } }
}
function global:Invoke-FixturePython {
    Write-Host 'FIXTURE python compile'
    $code = if ($Scenario -eq 'python-failure') { 7 } else { 0 }
    & $env:ComSpec /d /c "exit $code"
}
function global:npm {
    Write-Host "FIXTURE npm $args"
    if ($Scenario -eq 'throw') { throw 'fixture thrown command' }
    if ($Scenario -eq 'missing-command') { & 'worldos-fixture-command-does-not-exist'; return }
    $code = if (($Scenario -eq 'build-failure' -and $args[1] -eq 'build') -or ($Scenario -eq 'test-failure' -and $args[1] -eq 'test')) { 7 } else { 0 }
    & $env:ComSpec /d /c "exit $code"
}
function global:cargo {
    Write-Host 'FIXTURE cargo'
    if ($Scenario -eq 'native-warning') { & $env:ComSpec /d /c 'echo fixture native warning 1>&2 & exit 0'; return }
    & $env:ComSpec /d /c 'exit 7'
}
& "$PSScriptRoot/scripts/test.ps1" -Suite $Suite
exit $LASTEXITCODE
`;
      writeFileSync(path.join(dir, 'wrapper.ps1'), wrapper);
      const result = spawnSync(shell, ['-NoProfile', '-NonInteractive', '-File', path.join(dir, 'wrapper.ps1'), '-Scenario', scenario, '-Suite', suite], {
        cwd: dir, timeout: 15_000, maxBuffer: 65_536, encoding: 'utf8', windowsHide: true,
      });
      writeFileSync(path.join(dir, 'stdout.txt'), result.stdout || '');
      writeFileSync(path.join(dir, 'stderr.txt'), result.stderr || '');
      writeFileSync(path.join(dir, 'result.json'), JSON.stringify({ status: result.status, signal: result.signal, error: result.error?.message, sourceHash: hash(bytes), copyHash: hash(readFileSync(path.join(dir, 'scripts/test.ps1'))) }, null, 2));
      assert.ifError(result.error);
      assert.equal(result.signal, null);
      assert.equal(result.status, success ? 0 : 1, result.stdout + result.stderr);
      assert.equal(result.stdout.includes('All suites passed.'), success);
      assert.equal(hash(readFileSync(path.join(dir, 'scripts/test.ps1'))), hash(bytes));
      if (scenario === 'success' || scenario === 'aggregate-native-failure') {
        assert.match(result.stdout, /FIXTURE npm run build -w @worldos\/sdk/);
        assert.match(result.stdout, /FIXTURE npm run test -w @worldos\/sdk/);
        assert.match(result.stdout, /FIXTURE python compile/);
      }
      if (scenario === 'missing-deps') assert.match(result.stdout, /npm ci/);
      if (scenario === 'missing-python') assert.match(result.stdout, /Python/);
      if (scenario === 'alias-python') {
        assert.match(result.stdout, /Missing real Python/);
        assert.doesNotMatch(result.stdout, /FIXTURE python compile/);
      }
      if (scenario === 'native-warning') assert.match(result.stderr, /fixture native warning/);
      if (scenario === 'aggregate-native-failure') {
        assert.equal(result.stdout.split('FIXTURE cargo').length - 1, 2);
        assert.match(result.stdout, /Suites failed: cargo test --workspace, cli e2e/);
      }
      if (scenario === 'throw' || scenario === 'missing-command') {
        assert.match(result.stdout, /FAILED: ts sdk build/);
        assert.match(result.stdout, /FAILED: ts sdk test/);
        assert.match(result.stdout, /FIXTURE python compile/);
      }
    });
  }
  assert.equal(hash(readFileSync(source)), hash(bytes), 'Source changed during regression');
});
