import React from 'react'
import ReactDOM from 'react-dom/client'
import { BrowserRouter } from 'react-router-dom'
import '@arco-design/web-react/es/Alert/style/css.js'
import '@arco-design/web-react/es/Button/style/css.js'
import '@arco-design/web-react/es/Card/style/css.js'
import '@arco-design/web-react/es/DatePicker/style/css.js'
import '@arco-design/web-react/es/Descriptions/style/css.js'
import '@arco-design/web-react/es/Drawer/style/css.js'
import '@arco-design/web-react/es/Empty/style/css.js'
import '@arco-design/web-react/es/Form/style/css.js'
import '@arco-design/web-react/es/Grid/style/css.js'
import '@arco-design/web-react/es/Input/style/css.js'
import '@arco-design/web-react/es/InputNumber/style/css.js'
import '@arco-design/web-react/es/Message/style/css.js'
import '@arco-design/web-react/es/Modal/style/css.js'
import '@arco-design/web-react/es/Popconfirm/style/css.js'
import '@arco-design/web-react/es/Select/style/css.js'
import '@arco-design/web-react/es/Space/style/css.js'
import '@arco-design/web-react/es/Spin/style/css.js'
import '@arco-design/web-react/es/Statistic/style/css.js'
import '@arco-design/web-react/es/Switch/style/css.js'
import '@arco-design/web-react/es/Table/style/css.js'
import '@arco-design/web-react/es/Tabs/style/css.js'
import '@arco-design/web-react/es/Tag/style/css.js'
import App from './App'
import './index.css'

// Initialize theme from localStorage
const savedTheme = localStorage.getItem('vos-theme') || 'dark'
document.documentElement.setAttribute('data-theme', savedTheme)

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <BrowserRouter>
      <App />
    </BrowserRouter>
  </React.StrictMode>,
)
