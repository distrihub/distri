import { useMemo, useCallback } from 'react'
import {
  ReactFlow,
  Background,
  Controls,
  Node,
  Edge,
  NodeTypes,
  MarkerType,
  useNodesState,
  useEdgesState,
  Handle,
  Position,
} from '@xyflow/react'
import '@xyflow/react/dist/style.css'
import { Play, Square, Diamond, Repeat, GitBranch, Zap, Settings } from 'lucide-react'
import { WorkflowDAG, WorkflowEdge, WorkflowNode } from '@/utils/workflowUtils'

interface WorkflowDAGProps {
  dag: WorkflowDAG
  className?: string
}

// Custom node component for workflow nodes
const WorkflowNodeComponent = ({ data }: { data: any }) => {
  const getNodeIcon = () => {
    switch (data.nodeType) {
      case 'Start':
        return <Play className="h-4 w-4 text-green-600" />
      case 'End':
        return <Square className="h-4 w-4 text-red-600" />
      case 'AgentCall':
        return <Zap className="h-4 w-4 text-blue-600" />
      case 'ToolCall':
        return <Settings className="h-4 w-4 text-purple-600" />
      case 'Condition':
        return <Diamond className="h-4 w-4 text-yellow-600" />
      case 'Loop':
        return <Repeat className="h-4 w-4 text-orange-600" />
      case 'Parallel':
        return <GitBranch className="h-4 w-4 text-indigo-600" />
      default:
        return <Square className="h-4 w-4 text-gray-600" />
    }
  }

  const getNodeColor = () => {
    switch (data.nodeType) {
      case 'Start':
        return 'border-green-500 bg-green-50 dark:bg-green-800/30 text-green-800 dark:text-green-200'
      case 'End':
        return 'border-red-500 bg-red-50 dark:bg-red-800/30 text-red-800 dark:text-red-200'
      case 'AgentCall':
        return 'border-blue-500 bg-blue-50 dark:bg-blue-800/30 text-blue-800 dark:text-blue-200'
      case 'ToolCall':
        return 'border-purple-500 bg-purple-50 dark:bg-purple-800/30 text-purple-800 dark:text-purple-200'
      case 'Condition':
        return 'border-yellow-500 bg-yellow-50 dark:bg-yellow-800/30 text-yellow-800 dark:text-yellow-200'
      case 'Loop':
        return 'border-orange-500 bg-orange-50 dark:bg-orange-800/30 text-orange-800 dark:text-orange-200'
      case 'Parallel':
        return 'border-indigo-500 bg-indigo-50 dark:bg-indigo-800/30 text-indigo-800 dark:text-indigo-200'
      default:
        return 'border-gray-300 bg-white dark:bg-gray-800/30 text-gray-800 dark:text-gray-200'
    }
  }

  return (
    <div className={`px-4 py-2 shadow-md rounded-lg border-2 ${getNodeColor()} min-w-32`}>
      <Handle type="target" position={Position.Left} className="w-2 h-2" />
      <div className="flex items-center gap-2">
        {getNodeIcon()}
        <div className="text-sm font-medium">{data.label}</div>
      </div>
      {data.subtitle && (
        <div className="text-xs text-muted-foreground mt-1">{data.subtitle}</div>
      )}
      <Handle type="source" position={Position.Right} className="w-2 h-2" />
    </div>
  )
}

const nodeTypes: NodeTypes = {
  workflowNode: WorkflowNodeComponent,
}

export const WorkflowDAGComponent = ({ dag, className }: WorkflowDAGProps) => {
  // Convert API DAG format to ReactFlow format
  const { nodes: initialNodes, edges: initialEdges } = useMemo(() => {
    const nodes: Node[] = dag.nodes.map((node: WorkflowNode) => {
      const position = node.position ? { x: node.position[0], y: node.position[1] } : { x: 0, y: 0 }

      let subtitle = ''
      if (node.node_type.type === 'AgentCall' && node.node_type.function_name) {
        subtitle = `Agent: ${node.node_type.function_name}`
      } else if (node.node_type.type === 'ToolCall' && node.node_type.tool_name) {
        subtitle = `Tool: ${node.node_type.tool_name}`
      } else if (node.node_type.type === 'Condition' && node.node_type.expression) {
        subtitle = `If: ${node.node_type.expression}`
      } else if (node.node_type.type === 'Loop' && node.node_type.variable) {
        subtitle = `For: ${node.node_type.variable}`
      }

      return {
        id: node.id,
        type: 'workflowNode',
        position,
        data: {
          label: node.label,
          nodeType: node.node_type.type,
          subtitle,
          metadata: node.metadata,
        },
      }
    })

    const edges: Edge[] = dag.edges.map((edge: WorkflowEdge) => ({
      id: edge.id,
      source: edge.source,
      target: edge.target,
      label: edge.label,
      animated: false,
      markerEnd: {
        type: MarkerType.ArrowClosed,
        width: 15,
        height: 15,
      },
      style: {
        stroke: 'hsl(var(--muted-foreground))',
        strokeWidth: 2,
      },
    }))

    return { nodes, edges }
  }, [dag])

  const [nodes, , onNodesChange] = useNodesState(initialNodes)
  const [edges, , onEdgesChange] = useEdgesState(initialEdges)

  const onNodeClick = useCallback((event: React.MouseEvent, node: Node) => {
    console.log('Node clicked:', node)
    // TODO: Handle node click events (e.g., show node details, execute single node)
  }, [])

  const onEdgeClick = useCallback((event: React.MouseEvent, edge: Edge) => {
    console.log('Edge clicked:', edge)
    // TODO: Handle edge click events
  }, [])

  return (
    <div className={`h-full w-full ${className}`}>
      <ReactFlow
        nodes={nodes}
        edges={edges}
        onNodesChange={onNodesChange}
        onEdgesChange={onEdgesChange}
        onNodeClick={onNodeClick}
        onEdgeClick={onEdgeClick}
        nodeTypes={nodeTypes}
        fitView
        attributionPosition="bottom-left"
        minZoom={0.1}
        maxZoom={4}
        defaultViewport={{ x: 0, y: 0, zoom: 1 }}
      >
        <Background
          color="hsl(var(--muted-foreground) / 0.2)"
          gap={16}
        />
        <Controls />
      </ReactFlow>
    </div>
  )
}
