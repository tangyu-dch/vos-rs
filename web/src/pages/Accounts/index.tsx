import { useState, useEffect, useCallback } from 'react';
import {
  Card,
  Table,
  Button,
  Modal,
  Form,
  InputNumber,
  Message,
  Alert,
  Empty,
  Tag,
} from '@arco-design/web-react';
import { IconRefresh, IconPlus, IconSync } from '@arco-design/web-react/icon';
import { apiService } from '@/services/api';
import type { BillingAccount, LedgerEntry } from '@/types';

const FormItem = Form.Item;

function money(v: number): string {
  return `¥${(v || 0).toFixed(4)}`;
}

export default function Accounts() {
  const [accounts, setAccounts] = useState<BillingAccount[]>([]);
  const [ledger, setLedger] = useState<LedgerEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [creditUser, setCreditUser] = useState<string | null>(null);
  const [reconciling, setReconciling] = useState(false);
  const [form] = Form.useForm();

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [a, l] = await Promise.all([apiService.getAccounts(), apiService.getLedger()]);
      setAccounts(a);
      setLedger(l);
    } catch (err) {
      setError(err instanceof Error ? err.message : '加载失败');
      Message.error('加载账户数据失败');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const handleReconcile = async () => {
    setReconciling(true);
    try {
      const r = await apiService.reconcileBilling();
      Message.success(
        `对账完成：处理 ${r.processed} 条，跳过 ${r.skipped} 条，扣费 ${money(r.total_amount)}`
      );
      load();
    } catch {
      Message.error('对账失败');
    } finally {
      setReconciling(false);
    }
  };

  const handleCredit = async () => {
    try {
      const v = await form.validate();
      const r = await apiService.creditAccount(creditUser!, v.amount);
      Message.success(`充值成功，当前余额 ${money(r.balance)}`);
      setCreditUser(null);
      load();
    } catch {
      /* 校验失败 */
    }
  };

  const accountColumns = [
    {
      title: '账户',
      dataIndex: 'username',
      render: (v: string) => <span className="cell-mono cell-strong">{v}</span>,
    },
    {
      title: '余额',
      dataIndex: 'balance',
      width: 160,
      render: (v: number) => (
        <span className="cell-mono" style={{ fontWeight: 600, color: v < 0 ? 'var(--color-danger)' : 'var(--text-primary)' }}>
          {money(v)}
        </span>
      ),
    },
    { title: '币种', dataIndex: 'currency', width: 80 },
    {
      title: '状态',
      dataIndex: 'balance',
      width: 90,
      render: (v: number) =>
        v > 0 ? <Tag color="green">正常</Tag> : <Tag color="red">欠费</Tag>,
    },
    {
      title: '创建时间',
      dataIndex: 'created_at',
      render: (d: string) => (d ? new Date(d).toLocaleString('zh-CN') : '—'),
    },
    {
      title: '操作',
      dataIndex: 'actions',
      width: 110,
      fixed: 'right' as const,
      render: (_: any, record: BillingAccount) => (
        <Button
          type="text"
          size="small"
          icon={<IconPlus />}
          onClick={() => {
            setCreditUser(record.username);
            form.resetFields();
          }}
        >
          充值
        </Button>
      ),
    },
  ];

  const ledgerColumns = [
    {
      title: '时间',
      dataIndex: 'created_at',
      width: 180,
      render: (d: string) => (d ? new Date(d).toLocaleString('zh-CN') : '—'),
    },
    { title: '账户', dataIndex: 'username', width: 110, render: (v: string) => <span className="cell-mono">{v}</span> },
    { title: 'Call ID', dataIndex: 'call_id', ellipsis: true, render: (v: string) => <span className="cell-mono">{v}</span> },
    { title: '通话时长', dataIndex: 'duration_ms', width: 110, render: (v: number) => <span className="cell-mono">{Math.round(v / 1000)}s</span> },
    { title: '费率', dataIndex: 'rate_per_minute', width: 110, render: (v: number) => <span className="cell-mono">{money(v)}/分</span> },
    { title: '扣费', dataIndex: 'amount', width: 110, render: (v: number) => <span className="cell-mono" style={{ color: 'var(--color-danger)' }}>-{money(v)}</span> },
    { title: '扣后余额', dataIndex: 'balance_after', width: 120, render: (v: number) => <span className="cell-mono">{money(v)}</span> },
  ];

  return (
    <div className="page-wrap">
      <div className="page-header">
        <div className="page-header__title">
          <h1>账户与计费</h1>
          <span className="sub">账户余额、充值与离线对账扣费明细</span>
        </div>
        <div className="page-header__actions">
          <Button icon={<IconRefresh />} onClick={load}>
            刷新
          </Button>
          <Button
            type="primary"
            icon={<IconSync />}
            loading={reconciling}
            onClick={handleReconcile}
          >
            离线对账
          </Button>
        </div>
      </div>

      {error && <Alert type="error" content={error} closable style={{ marginBottom: 16 }} />}

      <Card className="app-card" bordered={false} title="账户">
        <Table
          className="app-table"
          columns={accountColumns}
          data={accounts}
          rowKey="username"
          loading={loading}
          pagination={false}
          noDataElement={<Empty description="暂无账户（充值时自动创建）" />}
        />
      </Card>

      <Card className="app-card" bordered={false} title="扣费明细" style={{ marginTop: 16 }}>
        <Table
          className="app-table"
          columns={ledgerColumns}
          data={ledger}
          rowKey="id"
          loading={loading}
          pagination={{ pageSize: 20, sizeCanChange: true }}
          noDataElement={<Empty description="暂无扣费记录（点击离线对账生成）" />}
        />
      </Card>

      <Modal
        title={`充值 - ${creditUser || ''}`}
        visible={!!creditUser}
        onOk={handleCredit}
        onCancel={() => setCreditUser(null)}
        okText="充值"
        cancelText="取消"
      >
        <Form form={form} labelCol={{ span: 5 }} wrapperCol={{ span: 19 }}>
          <FormItem
            label="充值金额"
            field="amount"
            required
            rules={[{ required: true, message: '请输入金额' }]}
          >
            <InputNumber placeholder="100.00" min={0.01} step={1} precision={2} style={{ width: '100%' }} />
          </FormItem>
        </Form>
      </Modal>
    </div>
  );
}
