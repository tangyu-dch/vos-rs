import { useState } from 'react';
import { Modal, ModalContent, ModalHeader, ModalBody, Chip, Button } from '@heroui/react';
import { Network, X } from 'lucide-react';
import { ResourceWorkspace } from '@/pages/shared/resource-workspace';
import { ivrMenus } from '@/pages/shared/resource-specs';
import { IvrTopologyEditor, type IvrFlowFields } from '@/components/ivr/ivr-rule-binding';
import type { Entity } from '@/services/resources';

export default function IvrPage() {
  const [topoIvr, setTopoIvr] = useState<IvrFlowFields | null>(null);
  const [workspaceKey, setWorkspaceKey] = useState(0);

  const handleTopologySaved = () => {
    setWorkspaceKey((prev) => prev + 1);
  };

  const ivrSpec = {
    ...ivrMenus,
    customRowAction: {
      label: '拓扑编排',
      icon: 'Network',
      color: 'primary' as const,
      onPress: (row: Entity) => {
        setTopoIvr({
          id: String(row.id ?? ''),
          name: String(row.name ?? ''),
          description: row.description ? String(row.description) : undefined,
          did: row.did ? String(row.did) : undefined,
          welcome_prompt: row.welcome_prompt ? String(row.welcome_prompt) : undefined,
          timeout_secs: row.timeout_secs ? Number(row.timeout_secs) : undefined,
          enabled: row.enabled !== false,
        });
      },
    },
  };

  return (
    <>
      <ResourceWorkspace key={workspaceKey} spec={ivrSpec} />

      {/* IVR 拓扑编排 Modal - 蓝图渐变 Banner 头部 */}
      <Modal
        isOpen={topoIvr !== null}
        onOpenChange={(o) => !o && setTopoIvr(null)}
        size="full"
        scrollBehavior="inside"
        hideCloseButton
        classNames={{
          base: 'h-screen max-h-screen w-screen max-w-screen',
          wrapper: 'h-screen max-h-screen',
        }}
      >
        <ModalContent className="h-full">
          <ModalHeader className="flex flex-col gap-0 p-0 border-b border-default-200 shrink-0 overflow-hidden">
            {/* 渐变 Banner */}
            <div className="relative bg-gradient-to-r from-primary/15 via-primary/5 to-transparent px-6 py-4 flex items-center justify-between gap-4">
              <div className="flex items-center gap-3 min-w-0">
                <div className="w-10 h-10 rounded-xl bg-primary/15 border border-primary/30 flex items-center justify-center shrink-0">
                  <Network className="w-5 h-5 text-primary" />
                </div>
                <div className="flex flex-col gap-0.5 min-w-0">
                  <div className="flex items-center gap-2">
                    <span className="text-base font-bold text-foreground">IVR 拓扑编排</span>
                    <span className="text-[10px] font-mono uppercase tracking-wider text-default-400 px-1.5 py-0.5 rounded bg-content2">Blueprint Editor</span>
                  </div>
                  {topoIvr && (
                    <div className="flex items-center gap-1.5 flex-wrap">
                      <Chip size="sm" variant="flat" color="primary" className="h-5 text-[10px] font-mono">
                        {topoIvr.id}
                      </Chip>
                      {topoIvr.did && (
                        <Chip size="sm" variant="flat" color="primary" className="h-5 text-[10px]">
                          DID {topoIvr.did}
                        </Chip>
                      )}
                    </div>
                  )}
                </div>
              </div>
              <Button
                isIconOnly
                size="sm"
                variant="light"
                onPress={() => setTopoIvr(null)}
                aria-label="关闭"
                className="shrink-0"
              >
                <X className="w-4 h-4 text-default-500" />
              </Button>
            </div>
          </ModalHeader>
          <ModalBody className="flex-1 min-h-0 p-4 overflow-hidden flex flex-col">
            {topoIvr && (
              <div className="flex-1 min-h-0">
                <IvrTopologyEditor flow={topoIvr} onSaved={handleTopologySaved} />
              </div>
            )}
          </ModalBody>
        </ModalContent>
      </Modal>
    </>
  );
}
