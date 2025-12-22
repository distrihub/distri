import { useState, useEffect } from 'react'
import { Header } from '@/components/ui/header'
import { WorkflowDAGComponent } from '@/components/WorkflowDAG'
import { WorkflowRunner } from '@/components/WorkflowRunner'
import { BACKEND_URL } from '@/constants'
import {
  convertSequentialWorkflowToDAG,
  getFirstStepType,
  SequentialWorkflowAgent,
  convertBackendDagToWorkflowDAG,
  BackendDag,
  WorkflowDAG
} from '@/utils/workflowUtils'

interface WorkflowDetailsViewProps {
  agent: any
}

export const WorkflowDetailsView = ({ agent }: WorkflowDetailsViewProps) => {
  const [agentDag, setAgentDag] = useState<WorkflowDAG | null>(null)

  // Fetch agent DAG for workflow agent types
  useEffect(() => {
    const fetchAgentDag = async () => {
      console.log('Agent loaded:', agent) // Debug log
      if (agent?.agent_type &&
        (agent.agent_type === 'sequential_workflow_agent' ||
          agent.agent_type === 'dag_workflow_agent' ||
          agent.agent_type === 'custom_agent')) {

        // Always try to fetch DAG from API first
        try {
          const response = await fetch(`${BACKEND_URL}/api/v1/agents/${agent.id}/dag`)
          console.log('DAG API response status:', response.status)
          if (response.ok) {
            const backendDag = await response.json() as BackendDag
            console.log('Backend DAG data:', backendDag)
            const workflowDag = convertBackendDagToWorkflowDAG(backendDag)
            console.log('Converted workflow DAG:', workflowDag)
            setAgentDag(workflowDag)
          } else {
            console.error('DAG API failed:', response.status, response.statusText)
            // Fallback: For sequential workflow agents, generate DAG from steps if available
            if (agent.agent_type === 'sequential_workflow_agent' && agent.steps) {
              const generatedDag = convertSequentialWorkflowToDAG(agent as SequentialWorkflowAgent)
              setAgentDag(generatedDag)
            }
          }
        } catch (error) {
          console.error('Error fetching agent DAG:', error)
          // Fallback: For sequential workflow agents, generate DAG from steps if available
          if (agent.agent_type === 'sequential_workflow_agent' && agent.steps) {
            const generatedDag = convertSequentialWorkflowToDAG(agent as SequentialWorkflowAgent)
            setAgentDag(generatedDag)
          }
        }
      }
    }

    if (agent) {
      fetchAgentDag()
    }
  }, [agent])

  return (
    <div className="flex flex-col h-screen relative" style={{ maxWidth: 'calc(100vw - 20rem)' }}>
      <Header
        leftElement={
          <>
            <h1 className="text-2xl font-bold">{agent?.name}</h1>
          </>
        }
        rightElement={null}
      />

      <div className="flex-1 flex overflow-hidden min-h-0 gap-2">
        <div className="flex-1 overflow-hidden">
          {agentDag ? (
            <div className="h-full flex">
              {/* DAG Visualization */}
              <div className="flex-1 p-4">
                <WorkflowDAGComponent dag={agentDag} className="h-full" />
              </div>
              {/* Workflow Runner Panel */}
              <div className="w-96 border-l bg-muted/20 p-4 overflow-y-auto">
                <WorkflowRunner
                  agent={agent}
                  firstStepType={agent.agent_type === 'sequential_workflow_agent' ? getFirstStepType(agent as SequentialWorkflowAgent) : 'agent'}
                />
              </div>
            </div>
          ) : (
            <div className="flex-1 flex items-center justify-center">
              <div className="text-center text-muted-foreground">
                <p>Loading workflow details...</p>
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  )
}