import test from 'node:test';
import assert from 'node:assert/strict';
import { PROVIDER_PRESETS, detectProvider, providerPreset } from '../src/domain/providers';

test('provider presets stay complete and honest about availability', () => {
  const ids = new Set(PROVIDER_PRESETS.map(preset => preset.id));
  assert.equal(ids.size, PROVIDER_PRESETS.length, '提供商 id 不得重复');
  for (const preset of PROVIDER_PRESETS) {
    // imap 与 pop3 都是手动填服务器，不携带预设地址。
    if (preset.available && !preset.oauth && preset.id !== 'imap' && preset.id !== 'pop3') {
      assert.ok(preset.endpoints, `${preset.id} 的密码 IMAP 预设必须提供服务器地址`);
      assert.equal(preset.endpoints.imapPort, 993);
      assert.equal(preset.endpoints.imapSecurity, 'tls');
      assert.ok(preset.endpoints.smtpPort, `${preset.id} 必须预填 SMTP`);
      assert.ok(preset.authHintKey, `${preset.id} 必须说明授权码获取方式`);
    }
    if (!preset.available) {
      assert.ok(!preset.endpoints, `${preset.id} 未开放时不应携带可保存的服务器预设`);
    }
  }
  const google = providerPreset('gmail');
  assert.equal(google.oauth, true, 'Gmail 必须走 OAuth 授权');
  assert.ok(!google.endpoints, 'OAuth 账号不应要求手填服务器');
});

test('detectProvider maps common domains to presets and falls back to generic IMAP', () => {
  assert.equal(detectProvider('user@Gmail.com'), 'gmail');
  assert.equal(detectProvider('user@googlemail.com'), 'gmail');
  assert.equal(detectProvider('user@qq.com'), 'qq');
  assert.equal(detectProvider('user@foxmail.com'), 'qq');
  assert.equal(detectProvider('user@163.com'), '163');
  assert.equal(detectProvider('user@126.com'), '163');
  assert.equal(detectProvider('user@outlook.com'), 'microsoft');
  assert.equal(detectProvider('user@example.test'), 'imap');
  assert.equal(detectProvider('not-an-email'), 'imap');
});
