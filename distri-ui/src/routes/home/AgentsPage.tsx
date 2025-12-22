import { useNavigate } from 'react-router-dom'
import AgentsHome from '@/components/AgentsHome'

const AgentsPage = () => {
  const navigate = useNavigate()

  return (
    <AgentsHome
      showDesignerChat={false}
      allowInlineCreator={false}
      onRequestCreateAgent={() => navigate('/home/new')}
    />
  )
}

export default AgentsPage
