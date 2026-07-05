/**
 * 从 SIP From/Contact/URI 中提取用户名部分。
 * 例: '"1001" <sip:1001@127.0.0.1:5060>;tag=xxx' -> '1001'
 *     '13800138000' -> '13800138000'
 */
export function extractSipUser(value?: string | null): string {
  if (!value) return '—';
  const m = value.match(/sip:([^@;>\s]+)/);
  return m ? m[1] : value;
}
