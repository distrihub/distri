// Define types locally since we removed workflowApi
export interface WorkflowNode {
  id: string
  node_type: {
    type: 'Start' | 'End' | 'AgentCall' | 'ToolCall' | 'Condition' | 'Loop' | 'Parallel'
    function_name?: string
    tool_name?: string
    expression?: string
    variable?: string
    iterable?: string
    branches?: string[]
  }
  label: string
  position?: [number, number]
  metadata: Record<string, any>
}

export interface WorkflowEdge {
  id: string
  source: string
  target: string
  label?: string
  condition?: string
}

export interface WorkflowDAG {
  nodes: WorkflowNode[]
  edges: WorkflowEdge[]
  layout?: string
}

export interface SequentialWorkflowStep {
  name: string
  type: 'tool_execution' | 'agent_execution'
  tool_name?: string
  agent_name?: string
  task: string
  input?: any
}

export interface SequentialWorkflowAgent {
  agent_type: string
  name: string
  description: string
  max_time?: number
  steps: SequentialWorkflowStep[]
}

export function convertSequentialWorkflowToDAG(agent: SequentialWorkflowAgent): WorkflowDAG {
  const nodes: WorkflowNode[] = []
  const edges: WorkflowEdge[] = []

  // Add start node
  nodes.push({
    id: 'start',
    node_type: {
      type: 'Start'
    },
    label: 'Start',
    position: [50, 100],
    metadata: {}
  })

  // Convert each step to a node
  agent.steps.forEach((step, index) => {
    const nodeId = step.name
    const nodeType = step.type === 'tool_execution' ? 'ToolCall' : 'AgentCall'
    
    nodes.push({
      id: nodeId,
      node_type: {
        type: nodeType as any,
        tool_name: step.tool_name,
        function_name: step.agent_name
      },
      label: step.name.replace(/_/g, ' ').replace(/\b\w/g, l => l.toUpperCase()),
      position: [200 + (index * 250), 100],
      metadata: {
        task: step.task,
        input: step.input
      }
    })

    // Add edge from previous node to this node
    const sourceId = index === 0 ? 'start' : agent.steps[index - 1].name
    edges.push({
      id: `${sourceId}-${nodeId}`,
      source: sourceId,
      target: nodeId,
      label: index === 0 ? 'Begin' : 'Next'
    })
  })

  // Add end node
  const lastStepId = agent.steps.length > 0 ? agent.steps[agent.steps.length - 1].name : 'start'
  nodes.push({
    id: 'end',
    node_type: {
      type: 'End'
    },
    label: 'End',
    position: [200 + (agent.steps.length * 250), 100],
    metadata: {}
  })

  edges.push({
    id: `${lastStepId}-end`,
    source: lastStepId,
    target: 'end',
    label: 'Complete'
  })

  return {
    nodes,
    edges,
    layout: 'horizontal'
  }
}

export function getFirstStepType(agent: SequentialWorkflowAgent): 'tool' | 'agent' | null {
  if (!agent.steps || agent.steps.length === 0) return null
  return agent.steps[0].type === 'agent_execution' ? 'agent' : 'tool'
}

// Backend DAG format
export interface BackendDagNode {
  id: string
  name: string
  node_type: string
  dependencies: string[]
  metadata: any
}

export interface BackendDag {
  nodes: BackendDagNode[]
  agent_name: string
  description: string
}

export function convertBackendDagToWorkflowDAG(backendDag: BackendDag): WorkflowDAG {
  const nodes: WorkflowNode[] = []
  const edges: WorkflowEdge[] = []

  // Convert backend nodes to workflow nodes
  backendDag.nodes.forEach((backendNode, index) => {
    nodes.push({
      id: backendNode.id,
      node_type: {
        type: backendNode.node_type === 'custom_agent' ? 'AgentCall' : 'ToolCall' as any,
        function_name: backendNode.name
      },
      label: backendNode.name.replace(/_/g, ' ').replace(/\b\w/g, l => l.toUpperCase()),
      position: [200 + (index * 300), 100],
      metadata: backendNode.metadata
    })

    // Create edges from dependencies
    backendNode.dependencies.forEach(depId => {
      edges.push({
        id: `${depId}-${backendNode.id}`,
        source: depId,
        target: backendNode.id,
        label: ''
      })
    })
  })

  // If there's only one node, add start and end nodes
  if (nodes.length === 1) {
    const singleNode = nodes[0]
    
    // Add start node
    nodes.unshift({
      id: 'start',
      node_type: { type: 'Start' },
      label: 'Start',
      position: [50, 100],
      metadata: {}
    })

    // Add end node
    nodes.push({
      id: 'end',
      node_type: { type: 'End' },
      label: 'End',
      position: [500, 100],
      metadata: {}
    })

    // Add edges
    edges.push({
      id: 'start-' + singleNode.id,
      source: 'start',
      target: singleNode.id,
      label: 'Begin'
    })

    edges.push({
      id: singleNode.id + '-end',
      source: singleNode.id,
      target: 'end',
      label: 'Complete'
    })
  }

  return {
    nodes,
    edges,
    layout: 'horizontal'
  }
}