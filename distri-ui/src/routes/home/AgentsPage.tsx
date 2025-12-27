import { useNavigate } from 'react-router-dom'
import AgentsHome from '@/components/AgentsHome'

const AgentsPage = () => {
  const navigate = useNavigate()

  return (
    <AgentsHome
      showDesignerChat={false}
      allowInlineCreator={false}
      onSelectAgent={(agent) => navigate(`/home/details?id=${encodeURIComponent(agent.id)}`)}
      onRequestCreateAgent={() => navigate('/home/new')}
    />
  )
}

export default AgentsPage
