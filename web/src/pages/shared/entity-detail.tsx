// 实体详情通用组件：根字段、Tabs 子资源加载、通话详情扩展
// 从 console.tsx 拆分

import { useCallback, useEffect, useState } from 'react';
import {
  Button, Card, CardBody, Spinner, Tabs, Tab,
} from '@heroui/react';
import { RefreshCw } from 'lucide-react';
import { useParams } from 'react-router-dom';
import { api } from '@/services/client';
import { getResource, type Entity } from '@/services/resources';
import { ErrorState } from '@/components/detail-shell';
import { callDetailLabel, callDetailText, valueText } from '@/pages/shared/format';
import {
  CallMediaDiagnostics, CallSummary, type CallMediaStatus, type SipFlowEvent, SipFlowDiagram,
} from '@/pages/shared/call-detail';

export function DetailFields({ value, empty }: { value: unknown; empty: string }) {
  if (!value || typeof value !== 'object') {
    return (
      <div className="py-8 text-center text-small text-default-400">{empty}</div>
    );
  }
  const entries = Object.entries(value as Entity);
  return (
    <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
      {entries.map(([label, fieldValue]) => (
        <div key={label} className="flex flex-col gap-1 p-3 rounded-medium bg-content2">
          <span className="text-tiny text-default-500 font-medium">{callDetailLabel(label)}</span>
          <span className="text-small text-foreground">{callDetailText(fieldValue, label)}</span>
        </div>
      ))}
    </div>
  );
}

interface EntityDetailTab {
  key: string;
  title: string;
  path?: string;
  sourceKey?: string;
}

export function EntityDetail({
  path, title, rootKey, tabs,
}: {
  path: string;
  title: string;
  rootKey?: string;
  tabs: EntityDetailTab[];
}) {
  const { id = '' } = useParams();
  const [entity, setEntity] = useState<Entity | null>(null);
  const [related, setRelated] = useState<Record<string, unknown>>({});
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');

  const load = useCallback(async () => {
    setLoading(true); setError('');
    try { setEntity(await getResource(path, id)); }
    catch (e) { setError(e instanceof Error ? e.message : '加载失败'); }
    finally { setLoading(false); }
  }, [id, path]);

  useEffect(() => { void load(); }, [load]);

  const loadTab = async (key: string, subpath?: string, sourceKey?: string) => {
    if (related[key] !== undefined) return;
    if (sourceKey && entity) {
      setRelated((old) => ({ ...old, [key]: entity[sourceKey] }));
      return;
    }
    if (!subpath) return;
    try {
      const value = subpath === 'recording'
        ? URL.createObjectURL(await api.blob(`${path}/${encodeURIComponent(id)}/${subpath}`))
        : await api.get(`${path}/${encodeURIComponent(id)}/${subpath}`);
      setRelated((old) => ({ ...old, [key]: value }));
    } catch (e) {
      setRelated((old) => ({ ...old, [key]: { error: e instanceof Error ? e.message : '加载失败' } }));
    }
  };

  const renderObject = (value: unknown, tabKey?: string) => {
    if (value === undefined) return <div className="py-10 flex justify-center"><Spinner color="primary" /></div>;
    if (typeof value === 'string' && value.startsWith('blob:')) {
      return <audio className="w-full" controls src={value} />;
    }
    if (value && typeof value === 'object' && 'error' in value) {
      return <ErrorState error={String((value as Entity).error)} retry={() => {
        setRelated((old) => { const next = { ...old }; delete next[tabKey!]; return next; });
      }} />;
    }
    if (tabKey === 'sipflow' && Array.isArray(value)) return <SipFlowDiagram events={value as SipFlowEvent[]} />;
    if (tabKey === 'media') {
      const mediaVal = value as CallMediaStatus;
      if (mediaVal.runtime_availability === 'not_active') {
        return <div className="py-8 text-center text-small text-default-400">通话已结束，实时媒体流已销毁</div>;
      }
      if (mediaVal.runtime_availability === 'unavailable') {
        return (
          <Card className="border border-warning-200 bg-warning-50 p-2">
            <CardBody className="p-4 text-small text-warning-700">实时媒体引擎不可达，无法获取指标</CardBody>
          </Card>
        );
      }
      return <CallMediaDiagnostics status={mediaVal} />;
    }
    const list = Array.isArray(value) ? value : [value];
    return (
      <div className="flex flex-col gap-3">
        {list.map((item, index) => <DetailFields key={index} value={item} empty="暂无数据" />)}
      </div>
    );
  };

  const root = rootKey && entity?.[rootKey] && typeof entity[rootKey] === 'object' ? entity[rootKey] as Entity : entity;
  const headerTitle = root ? valueText(root.name || root.username || root.id || id) : title;

  return (
    <div className="flex flex-col gap-4">
      <Card shadow="sm" className="p-2">
        <CardBody className="p-4 flex flex-wrap items-center justify-between gap-4">
          <div>
            <h1 className="text-base font-bold text-foreground">{headerTitle}</h1>
            <p className="text-tiny text-default-500 mt-0.5">{title}详情与关联运行状态。</p>
          </div>
          <Button variant="flat" size="sm" isLoading={loading} onPress={load} startContent={<RefreshCw className="w-4 h-4" />}>
            刷新
          </Button>
        </CardBody>
      </Card>

      {error ? (
        <ErrorState error={error} retry={load} />
      ) : loading ? (
        <div className="py-20 flex justify-center"><Spinner color="primary" label="加载中..." /></div>
      ) : entity ? (
        <Card shadow="sm" className="p-2">
          <CardBody className="p-4">
            <Tabs
              aria-label={`${title}详情`}
              onSelectionChange={(key) => {
                const tab = tabs.find((t) => t.key === key);
                if (tab && related[tab.key] === undefined) void loadTab(tab.key, tab.path, tab.sourceKey);
              }}
            >
              {tabs.map((tab) => (
                <Tab key={tab.key} title={tab.title}>
                  <div className="py-4">
                    {tab.key === 'summary' && path === '/calls' ? <CallSummary entity={entity} />
                      : tab.key === 'summary' && root ? <DetailFields value={root} empty="暂无详情" />
                      : renderObject(related[tab.key], tab.key)}
                  </div>
                </Tab>
              ))}
            </Tabs>
          </CardBody>
        </Card>
      ) : null}
    </div>
  );
}
