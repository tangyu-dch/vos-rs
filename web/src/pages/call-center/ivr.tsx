import { useState } from 'react';
import { Modal, ModalContent, ModalHeader, ModalBody, Chip } from '@heroui/react';
import { Network } from 'lucide-react';
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

      {/* IVR 拓扑编排 Modal */}
      <Modal
        isOpen={topoIvr !== null}
        onOpenChange={(o) => !o && setTopoIvr(null)}
        size="full"
        scrollBehavior="inside"
        classNames={{
          base: 'h-screen max-h-screen w-screen max-w-screen',
          wrapper: 'h-screen max-h-screen',
        }}
      >
        <ModalContent className="h-full">
          <ModalHeader className="flex items-center gap-2 border-b border-default-200 dark:border-slate-800 shrink-0">
            <Network className="w-5 h-5 text-primary-600" />
            <span>IVR 拓扑编排</span>
            {topoIvr && (
              <>
                <Chip size="sm" variant="flat" color="primary" className="ml-2">
                  {topoIvr.id}
                </Chip>
                {topoIvr.did && (
                  <Chip size="sm" variant="flat" color="primary">DID {topoIvr.did}</Chip>
                )}
              </>
            )}
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
