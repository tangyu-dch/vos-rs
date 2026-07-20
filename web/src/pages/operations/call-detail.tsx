// 运营监控 - 通话详情
// 从 console.tsx 拆分

import { EntityDetail } from '@/pages/shared/entity-detail';

export const CallDetailPage = () => (
  <EntityDetail
    path="/calls"
    title="通话"
    tabs={[
      { key: 'summary', title: '呼叫概览' },
      { key: 'sipflow', title: '信令流图', path: 'sipflow' },
      { key: 'media', title: '媒体指标', path: 'media' },
      { key: 'dtmf', title: 'DTMF', path: 'dtmf' },
      { key: 'recording', title: '录音', path: 'recording' },
    ]}
  />
);
