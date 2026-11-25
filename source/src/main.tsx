import React from 'react';
import { createRoot } from 'react-dom/client';
import App from './WebApp';
import './styles.css';
import './ui.css';
import './web.css';
createRoot(document.getElementById('root')!).render(<React.StrictMode><App /></React.StrictMode>);
