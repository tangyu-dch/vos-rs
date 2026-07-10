import { useState, useEffect, useRef } from 'react';
import {
  Card,
  Grid,
  Button,
  Space,
  Statistic,
  Spin,
  Alert,
  Empty,
  Tag,
  Message,
} from '@arco-design/web-react';
import { IconSearch, IconDownload } from '@arco-design/web-react/icon';
import { apiService } from '@/services/api';
import type { ReportSummary } from '@/types';
import { graphic, init, type ECharts } from '@/utils/charts';

const { Row, Col } = Grid;

const STATUS_LABEL: Record<string, string> = {
  answered: '已接通',
  canceled: '已取消',
  failed: '失败',
};
const STATUS_COLOR: Record<string, string> = {
  answered: 'var(--status-online)',
  canceled: 'var(--status-break)',
  failed: 'var(--color-danger)',
};

// Theme-aware ECharts colors
function getThemeColors() {
  const style = getComputedStyle(document.documentElement);
  return {
    textMuted: style.getPropertyValue('--text-muted').trim() || '#5c5f72',
    textSecondary: style.getPropertyValue('--text-secondary').trim() || '#a0a3b5',
    borderSubtle: style.getPropertyValue('--border-subtle').trim() || 'rgba(255,255,255,0.06)',
    accent: style.getPropertyValue('--accent').trim() || '#3ee8c8',
    cardBg: style.getPropertyValue('--card-bg').trim() || '#0f1117',
    textPrimary: style.getPropertyValue('--text-primary').trim() || '#f0f1f5',
  };
}

function quickRange(label: string): { start?: string; end?: string } {
  const now = new Date();
  switch (label) {
    case '今天': {
      const d = new Date(); d.setHours(0, 0, 0, 0);
      return { start: d.toISOString(), end: now.toISOString() };
    }
    case '最近 7 天': {
      const s = new Date(); s.setDate(s.getDate() - 7);
      return { start: s.toISOString(), end: now.toISOString() };
    }
    case '最近 30 天': {
      const s = new Date(); s.setDate(s.getDate() - 30);
      return { start: s.toISOString(), end: now.toISOString() };
    }
    default:
      return {};
  }
}

export default function Reports() {
  const [summary, setSummary] = useState<ReportSummary | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [activeRange, setActiveRange] = useState('最近 7 天');

  const trendRef = useRef<HTMLDivElement>(null);
  const trendChart = useRef<ECharts | null>(null);
  const pieRef = useRef<HTMLDivElement>(null);
  const pieChart = useRef<ECharts | null>(null);
  const durationRef = useRef<HTMLDivElement>(null);
  const durationChart = useRef<ECharts | null>(null);
  const mosRef = useRef<HTMLDivElement>(null);
  const mosChart = useRef<ECharts | null>(null);

  const load = async (opts?: { start?: string; end?: string }) => {
    setLoading(true);
    setError(null);
    try {
      const s = await apiService.getReportSummary(opts?.start, opts?.end);
      setSummary(s);
    } catch (err) {
      setError(err instanceof Error ? err.message : '加载失败');
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => { load(); }, []);

  const handleQuickRange = (label: string) => {
    setActiveRange(label);
    load(label === '全部' ? undefined : quickRange(label));
  };

  useEffect(() => {
    if (!summary) return;

    const days = summary.by_day.map(d => d.day);
    const dayMap = new Map(summary.by_day.map(d => [d.day, d]));
    const fullDays = (() => {
      const startStr = summary.start.slice(0, 10);
      const endStr = summary.end.slice(0, 10);
      if (!startStr || !endStr) return days;
      const start = new Date(startStr);
      const end = new Date(endStr);
      const result: string[] = [];
      const cur = new Date(start);
      while (cur <= end) {
        result.push(cur.toISOString().slice(0, 10));
        cur.setDate(cur.getDate() + 1);
      }
      return result.length > 0 ? result : days;
    })();
    const totalData = fullDays.map(d => dayMap.get(d)?.total ?? 0);
    const answeredData = fullDays.map(d => dayMap.get(d)?.answered ?? 0);

    if (trendRef.current) {
      trendChart.current ||= init(trendRef.current);
    const tc = getThemeColors();
    trendChart.current.setOption({
      tooltip: { trigger: 'axis', backgroundColor: tc.cardBg, borderColor: tc.borderSubtle, textStyle: { color: tc.textPrimary } },
      legend: { data: ['总呼叫', '已接通'], bottom: 0, textStyle: { color: tc.textSecondary } },
      grid: { left: 50, right: 16, top: 20, bottom: 40 },
      xAxis: { type: 'category', data: fullDays, axisLabel: { color: tc.textMuted, rotate: fullDays.length > 14 ? 30 : 0, formatter: (v: string) => v.slice(5) } },
      yAxis: { type: 'value', splitLine: { lineStyle: { color: tc.borderSubtle } }, axisLabel: { color: tc.textMuted }, minInterval: 1 },
      series: [
        {
          name: '总呼叫', type: 'line', smooth: true, symbol: 'circle', symbolSize: 6,
          areaStyle: { color: new graphic.LinearGradient(0, 0, 0, 1, [
            { offset: 0, color: 'rgba(129,140,248,0.25)' }, { offset: 1, color: 'rgba(129,140,248,0.01)' },
          ]) },
          lineStyle: { color: '#818cf8', width: 2 },
          itemStyle: { color: '#818cf8' },
          data: totalData,
        },
        {
          name: '已接通', type: 'line', smooth: true, symbol: 'circle', symbolSize: 6,
          areaStyle: { color: new graphic.LinearGradient(0, 0, 0, 1, [
            { offset: 0, color: 'rgba(34,211,238,0.25)' }, { offset: 1, color: 'rgba(34,211,238,0.01)' },
          ]) },
          lineStyle: { color: '#22d3ee', width: 2 },
          itemStyle: { color: '#22d3ee' },
          data: answeredData,
        },
      ],
    });
    }

    if (pieRef.current) {
      const tc = getThemeColors();
      pieChart.current ||= init(pieRef.current);
      pieChart.current.setOption({
        tooltip: { trigger: 'item', backgroundColor: tc.cardBg, borderColor: tc.borderSubtle, textStyle: { color: tc.textPrimary } },
        legend: { bottom: 0, icon: 'circle', textStyle: { color: tc.textSecondary } },
        series: [{
          type: 'pie', radius: ['50%', '72%'], center: ['50%', '45%'],
          itemStyle: { borderRadius: 6, borderColor: tc.cardBg, borderWidth: 3 },
          label: { show: false },
          data: summary.by_status.map(s => ({
            name: STATUS_LABEL[s.status] || s.status,
            value: s.count,
            itemStyle: { color: STATUS_COLOR[s.status] || tc.textMuted },
          })),
        }],
      });
    }

    if (durationRef.current) {
      const tc = getThemeColors();
      durationChart.current ||= init(durationRef.current);
      durationChart.current.setOption({
        tooltip: { trigger: 'axis', backgroundColor: tc.cardBg, borderColor: tc.borderSubtle, textStyle: { color: tc.textPrimary }, formatter: (p: any) => `${p[0]?.name}<br/>${p[0]?.marker} 平均通话: ${p[0]?.value} 秒` },
        grid: { left: 80, right: 16, top: 16, bottom: 40 },
        xAxis: {
          type: 'category', data: fullDays,
          axisLabel: { color: tc.textMuted, rotate: fullDays.length > 14 ? 30 : 0, formatter: (v: string) => v.slice(5) },
        },
        yAxis: {
          type: 'value', name: '秒',
          splitLine: { lineStyle: { color: tc.borderSubtle } },
          axisLabel: { color: tc.textMuted },
        },
        series: [{
          type: 'line', smooth: true, symbol: 'circle', symbolSize: 6,
          areaStyle: { color: new graphic.LinearGradient(0, 0, 0, 1, [
            { offset: 0, color: 'rgba(129,140,248,0.3)' },
            { offset: 1, color: 'rgba(129,140,248,0.02)' },
          ]) },
          lineStyle: { color: '#818cf8', width: 2 },
          itemStyle: { color: '#818cf8' },
          data: fullDays.map(d => {
            const day = dayMap.get(d);
            if (!day || day.answered === 0) return 0;
            return Math.round(summary.total_duration_ms / Math.max(summary.answered, 1) / 1000);
          }),
        }],
      });
    }

    if (mosRef.current && summary.avg_mos) {
      const tc = getThemeColors();
      mosChart.current ||= init(mosRef.current);
      mosChart.current.setOption({
        series: [{
          type: 'gauge',
          startAngle: 200, endAngle: -20,
          min: 0, max: 5, splitNumber: 5, radius: '90%',
          axisLine: { lineStyle: { width: 12, color: [[0.3, '#f87171'], [0.7, '#fbbf24'], [0.9, '#facc15'], [1, '#34d399']] } },
          pointer: { width: 4, length: '60%', itemStyle: { color: tc.textPrimary } },
          axisTick: { show: false },
          splitLine: { length: 12, lineStyle: { width: 2, color: tc.textMuted } },
          axisLabel: { distance: 16, color: tc.textMuted, fontSize: 12 },
          detail: { valueAnimation: true, formatter: '{value}', color: tc.textPrimary, fontSize: 28, fontWeight: 'bold', offsetCenter: [0, '70%'] },
          title: { offsetCenter: [0, '95%'], color: tc.textMuted, fontSize: 14 },
          data: [{ value: Number(summary.avg_mos.toFixed(1)), name: '平均 MOS' }],
        }],
      });
    }
  }, [summary]);

  useEffect(() => {
    const onResize = () => {
      trendChart.current?.resize();
      pieChart.current?.resize();
      durationChart.current?.resize();
      mosChart.current?.resize();
    };
    window.addEventListener('resize', onResize);
    return () => {
      window.removeEventListener('resize', onResize);
      trendChart.current?.dispose();
      pieChart.current?.dispose();
      durationChart.current?.dispose();
      mosChart.current?.dispose();
    };
  }, []);

  const handleExport = async () => {
    const opts = activeRange === '全部' ? undefined : quickRange(activeRange);
    try {
      const blob = await apiService.exportReport(opts?.start, opts?.end);
      const url = URL.createObjectURL(blob);
      const link = document.createElement('a');
      link.href = url;
      link.download = 'vos-report.csv';
      link.click();
      URL.revokeObjectURL(url);
    } catch {
      Message.error('导出报表失败');
    }
  };

  if (loading && !summary) {
    return <div className="loading-wrap"><Spin size={32} /><span>加载报表…</span></div>;
  }

  const answerRate = summary ? (summary.answered / Math.max(summary.total, 1) * 100) : 0;
  const avgDuration = summary ? Math.round(summary.total_duration_ms / Math.max(summary.answered, 1) / 1000) : 0;

  return (
    <div className="page-wrap">
      <div className="page-header">
        <div className="page-header__title">
          <h1>报表</h1>
        </div>
        <div className="page-header__actions">
          <Button icon={<IconDownload />} onClick={handleExport}>导出 CSV</Button>
        </div>
      </div>

      <Card className="app-card" bordered={false} style={{ marginBottom: 16 }}>
        <Space wrap size={8}>
          {['今天', '最近 7 天', '最近 30 天', '全部'].map(label => (
            <Button
              key={label}
              type={activeRange === label ? 'primary' : 'outline'}
              onClick={() => handleQuickRange(label)}
            >
              {label}
            </Button>
          ))}
          <Button type="primary" icon={<IconSearch />} onClick={() => load()}>刷新</Button>
        </Space>
      </Card>

      {error && <Alert type="error" content={error} closable style={{ marginBottom: 16 }} />}
      {loading && <Spin style={{ display: 'block', margin: '40px auto' }} />}

      {summary && (
        <>
          <Row gutter={[12, 12]}>
            <Col span={3}><Card className="app-card" bordered={false}><Statistic title="总呼叫" value={summary.total} suffix="次" /></Card></Col>
            <Col span={3}><Card className="app-card" bordered={false}><Statistic title="已接通" value={summary.answered} suffix="次" /></Card></Col>
            <Col span={3}><Card className="app-card" bordered={false}><Statistic title="失败/取消" value={summary.failed + summary.canceled} suffix="次" /></Card></Col>
            <Col span={3}><Card className="app-card" bordered={false}><Statistic title="接通率" value={answerRate.toFixed(1)} suffix="%" /></Card></Col>
            <Col span={3}><Card className="app-card" bordered={false}><Statistic title="平均振铃" value={summary.avg_ring_ms != null ? (summary.avg_ring_ms / 1000).toFixed(1) : '—'} suffix="秒" /></Card></Col>
            <Col span={3}><Card className="app-card" bordered={false}><Statistic title="平均通话" value={avgDuration} suffix="秒" /></Card></Col>
            <Col span={3}><Card className="app-card" bordered={false}><Statistic title="总时长" value={Math.round(summary.total_duration_ms / 60000)} suffix="分钟" /></Card></Col>
            <Col span={3}><Card className="app-card" bordered={false}><Statistic title="MOS" value={summary.avg_mos != null ? summary.avg_mos.toFixed(1) : '—'} /></Card></Col>
          </Row>

          <Row gutter={[12, 12]} style={{ marginTop: 12 }}>
            <Col span={4}><Card className="app-card" bordered={false}><Statistic title="信令延迟 (RTT)" value={summary.avg_rtt_ms != null ? summary.avg_rtt_ms.toFixed(0) : '—'} suffix="ms" /></Card></Col>
            <Col span={4}><Card className="app-card" bordered={false}><Statistic title="丢包率" value={summary.avg_loss_rate != null ? summary.avg_loss_rate.toFixed(2) : '—'} suffix="%" /></Card></Col>
            <Col span={4}><Card className="app-card" bordered={false}><Statistic title="抖动" value={summary.avg_jitter_ms != null ? summary.avg_jitter_ms.toFixed(1) : '—'} suffix="ms" /></Card></Col>
            <Col span={4}><Card className="app-card" bordered={false}><Statistic title="呼叫建立" value={summary.avg_setup_ms != null ? (summary.avg_setup_ms / 1000).toFixed(1) : '—'} suffix="秒" /></Card></Col>
            <Col span={4}><Card className="app-card" bordered={false}><Statistic title="总通话" value={Math.round(summary.total_duration_ms / 1000)} suffix="秒" /></Card></Col>
            <Col span={4}><Card className="app-card" bordered={false}><Statistic title="计费时长" value={Math.round(summary.total_billable_ms / 1000)} suffix="秒" /></Card></Col>
          </Row>

          <Row gutter={[16, 16]} style={{ marginTop: 16 }}>
            <Col span={12}>
              <Card className="app-card" bordered={false} title="每日呼叫量">
                {summary.by_day.length ? <div ref={trendRef} style={{ height: 300 }} /> : <Empty description="所选区间无数据" />}
              </Card>
            </Col>
            <Col span={12}>
              <Card className="app-card" bordered={false} title="状态分布">
                {summary.by_status.length ? <div ref={pieRef} style={{ height: 300 }} /> : <Empty description="无数据" />}
              </Card>
            </Col>
          </Row>

          <Row gutter={[16, 16]} style={{ marginTop: 16 }}>
            <Col span={12}>
              <Card className="app-card" bordered={false} title="通话时长趋势">
                {summary.by_day.length ? <div ref={durationRef} style={{ height: 260 }} /> : <Empty description="无数据" />}
              </Card>
            </Col>
            <Col span={12}>
              <Card className="app-card" bordered={false} title="MOS 质量评分">
                {summary.avg_mos ? <div ref={mosRef} style={{ height: 260 }} /> : <Empty description="无 MOS 数据" />}
              </Card>
            </Col>
          </Row>

          <Card className="app-card" bordered={false} style={{ marginTop: 16 }} title="状态明细">
            <table style={{ width: '100%', borderCollapse: 'collapse' }}>
              <thead>
                <tr style={{ borderBottom: '1px solid var(--border-subtle)', textAlign: 'left' }}>
                  <th style={{ padding: '8px 12px', color: 'var(--text-muted)', fontSize: 13 }}>状态</th>
                  <th style={{ padding: '8px 12px', color: 'var(--text-muted)', fontSize: 13 }}>数量</th>
                  <th style={{ padding: '8px 12px', color: 'var(--text-muted)', fontSize: 13 }}>占比</th>
                  <th style={{ padding: '8px 12px', color: 'var(--text-muted)', fontSize: 13 }}>总时长</th>
                  <th style={{ padding: '8px 12px', color: 'var(--text-muted)', fontSize: 13 }}>平均时长</th>
                </tr>
              </thead>
              <tbody>
                {summary.by_status.map(s => (
                  <tr key={s.status} style={{ borderBottom: '1px solid var(--border-subtle)' }}>
                    <td style={{ padding: '8px 12px' }}><Tag color={STATUS_COLOR[s.status] || 'gray'}>{STATUS_LABEL[s.status] || s.status}</Tag></td>
                    <td style={{ padding: '8px 12px', fontFamily: 'monospace' }}>{s.count}</td>
                    <td style={{ padding: '8px 12px', fontFamily: 'monospace' }}>{(s.count / Math.max(summary.total, 1) * 100).toFixed(1)}%</td>
                    <td style={{ padding: '8px 12px', fontFamily: 'monospace' }}>{Math.round(s.duration_ms / 60000)} 分钟</td>
                    <td style={{ padding: '8px 12px', fontFamily: 'monospace' }}>{s.count > 0 ? Math.round(s.duration_ms / s.count / 1000) : 0} 秒</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </Card>
        </>
      )}
    </div>
  );
}
