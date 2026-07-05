import { useState, useEffect, useRef } from 'react';
import {
  Card,
  Grid,
  DatePicker,
  Button,
  Space,
  Statistic,
  Spin,
  Alert,
  Empty,
} from '@arco-design/web-react';
import { IconSearch, IconDownload } from '@arco-design/web-react/icon';
import * as echarts from 'echarts';
import { apiService } from '@/services/api';
import type { ReportSummary } from '@/types';

const { RangePicker } = DatePicker;
const { Row, Col } = Grid;

const STATUS_LABEL: Record<string, string> = {
  answered: '已接通',
  canceled: '已取消',
  failed: '失败',
};
const STATUS_COLOR: Record<string, string> = {
  answered: '#00b42a',
  canceled: '#ff7d00',
  failed: '#f53f3f',
};

export default function Reports() {
  const [summary, setSummary] = useState<ReportSummary | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [range, setRange] = useState<[string, string] | []>([]);

  const trendRef = useRef<HTMLDivElement>(null);
  const trendChart = useRef<echarts.ECharts | null>(null);
  const pieRef = useRef<HTMLDivElement>(null);
  const pieChart = useRef<echarts.ECharts | null>(null);

  const load = async () => {
    setLoading(true);
    setError(null);
    try {
      const s = await apiService.getReportSummary(range[0], range[1]);
      setSummary(s);
    } catch (err) {
      setError(err instanceof Error ? err.message : '加载失败');
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (!summary) return;
    if (trendRef.current) {
      trendChart.current ||= echarts.init(trendRef.current);
      trendChart.current.setOption({
        tooltip: { trigger: 'axis' },
        legend: { data: ['总呼叫', '已接通'], bottom: 0 },
        grid: { left: 36, right: 16, top: 20, bottom: 40 },
        xAxis: { type: 'category', data: summary.by_day.map((d) => d.day), axisLabel: { color: '#86909c' } },
        yAxis: { type: 'value', splitLine: { lineStyle: { color: '#f0f1f3' } }, axisLabel: { color: '#86909c' } },
        series: [
          { name: '总呼叫', type: 'bar', data: summary.by_day.map((d) => d.total), itemStyle: { color: '#165dff', borderRadius: [4, 4, 0, 0] } },
          { name: '已接通', type: 'bar', data: summary.by_day.map((d) => d.answered), itemStyle: { color: '#0fc6c2', borderRadius: [4, 4, 0, 0] } },
        ],
      });
    }
    if (pieRef.current) {
      pieChart.current ||= echarts.init(pieRef.current);
      pieChart.current.setOption({
        tooltip: { trigger: 'item' },
        legend: { bottom: 0, icon: 'circle', textStyle: { color: '#4e5969' } },
        series: [
          {
            type: 'pie',
            radius: ['50%', '72%'],
            center: ['50%', '45%'],
            itemStyle: { borderRadius: 6, borderColor: '#fff', borderWidth: 3 },
            label: { show: false },
            data: summary.by_status.map((s) => ({
              name: STATUS_LABEL[s.status] || s.status,
              value: s.count,
              itemStyle: { color: STATUS_COLOR[s.status] || '#86909c' },
            })),
          },
        ],
      });
    }
  }, [summary]);

  useEffect(() => {
    const onResize = () => {
      trendChart.current?.resize();
      pieChart.current?.resize();
    };
    window.addEventListener('resize', onResize);
    return () => {
      window.removeEventListener('resize', onResize);
      trendChart.current?.dispose();
      pieChart.current?.dispose();
    };
  }, []);

  const handleExport = () => {
    window.open(apiService.reportExportUrl(range[0], range[1]));
  };

  if (loading && !summary) {
    return (
      <div className="loading-wrap">
        <Spin size={32} />
        <span>加载报表…</span>
      </div>
    );
  }

  return (
    <div className="page-wrap">
      <div className="page-header">
        <div className="page-header__title">
          <h1>报表</h1>
          <span className="sub">按时段统计呼叫量、时长与质量，支持 CSV 导出</span>
        </div>
        <div className="page-header__actions">
          <Button icon={<IconDownload />} onClick={handleExport}>
            导出 CSV
          </Button>
        </div>
      </div>

      <Card className="app-card" bordered={false} style={{ marginBottom: 16 }}>
        <Space wrap>
          <RangePicker
            showTime
            style={{ width: 360 }}
            value={range as any}
            onChange={(v) => setRange((v || []) as [string, string] | [])}
          />
          <Button type="primary" icon={<IconSearch />} onClick={load}>
            查询
          </Button>
        </Space>
      </Card>

      {error && <Alert type="error" content={error} closable style={{ marginBottom: 16 }} />}

      {summary && (
        <>
          <Row gutter={[16, 16]}>
            <Col span={6}>
              <Card className="app-card" bordered={false}>
                <Statistic title="总呼叫" value={summary.total} />
              </Card>
            </Col>
            <Col span={6}>
              <Card className="app-card" bordered={false}>
                <Statistic title="已接通" value={summary.answered} suffix="次" />
              </Card>
            </Col>
            <Col span={6}>
              <Card className="app-card" bordered={false}>
                <Statistic title="失败/取消" value={summary.failed + summary.canceled} suffix="次" />
              </Card>
            </Col>
            <Col span={6}>
              <Card className="app-card" bordered={false}>
                <Statistic
                  title="总通话时长"
                  value={Math.round(summary.total_duration_ms / 60000)}
                  suffix="分钟"
                />
              </Card>
            </Col>
          </Row>

          <Row gutter={[16, 16]} style={{ marginTop: 16 }}>
            <Col span={16}>
              <Card className="app-card" bordered={false} title="每日呼叫量">
                {summary.by_day.length ? (
                  <div ref={trendRef} style={{ height: 300 }} />
                ) : (
                  <Empty description="所选区间无数据" />
                )}
              </Card>
            </Col>
            <Col span={8}>
              <Card className="app-card" bordered={false} title="状态分布">
                {summary.by_status.length ? (
                  <div ref={pieRef} style={{ height: 300 }} />
                ) : (
                  <Empty description="无数据" />
                )}
              </Card>
            </Col>
          </Row>
        </>
      )}
    </div>
  );
}
