# Handoff: ctx

- **Created:** 2026-09-05T00:15:53.884+00:00
- **Source Agent:** claude
- **Source Machine:** Mac-mini.local
- **Git Branch:** main

## Summary

Listo en ambos lados. Ahora hay un desplegable **"📋 Pegar un JSON existente para verlo"** arriba del formulario — pega ahí el JSON (o el mensaje de chat completo, con fecha y nombre incluidos, como el que me mandaste) y dale "Cargar en el formulario". Detecta automáticamente si es tipo lista o botones, llena todos los campos y muestra el preview. Si algún campo del JSON pegado excede el límite de Meta, lo recorta y te avisa exactamente cuál y por cuánto — probé el tuyo y cabía todo sin recortes.

audit log + 30-day tokens

## Tasks

### Completed

- [x] Investigar a fondo el bug de la máquina de estados de maipa (comparar con pymes/flow_engine.py que sí funciona)
- [x] Investigar esquema de roles en maipa/comercial para el fix de autorización
- [x] Fase 1 (crítico): SSRF en licitaciones + clasificador clínico muerto en maipa + rol en comercial + auth en mark-handled
- [x] Fase 2 (alto): máquina de estados de maipa + XSS licitaciones + dedup atómico fintech
- [x] Fase 4: decisión tomada — no tocar código muerto de Bloqueo, dejar documentado en el reporte final
- [x] Fase 3 (medio): regresión pymes cashflow/inventario/ventas + historial duplicado + logs Supabase + otros
- [x] Fase 2 (alto): XSS licitaciones + dedup atómico fintech
- [x] Fase 4: código muerto de Bloqueo — auditado y documentado, decisión pendiente del usuario
- [x] Simplificar maipa/flow_engine.py eliminando todo el menú de negocio duplicado de pymes (no usado, confirmado por el usuario)
- [x] Corregir regresión clínica encontrada por el auditor (default a 'sospechoso' en vez de 'normal' para valores desconocidos)
- [x] Auditoría independiente final: fixes 1-3 + simplificación de maipa — verificado, sin bugs críticos
- [x] Auditar conversaciones de julio (CommandCode + opencode) para encontrar errores nuevos no reportados
- [x] Verificar contra documentación oficial de YCloud el bug de botón sin body.text
- [x] Fix B: degradar botón sin body.text a texto plano en vez de perderlo (ycloud.py)
- [x] Fix C: contador de repetición del override de titularidad, escalando a humano tras 2 repeticiones
- [x] Fix A: derivación determinística a humano cuando el LLM detecta intent hablar_asesor/escala_humano/etc
- [x] Investigar sistema de diseño de TuConsejería (skill infografias: colores, tipografía Rubik)
- [x] Investigar arquitectura de llamadas de voz (proyecto TuConfIA: Zadarma + ElevenLabs)
- [x] Investigar precios reales de ElevenLabs 2026 para tabla de minutos con margen 50%
- [x] Investigar cambio de cobro por mensaje de Meta desde octubre 2026
- [x] Revisar documento de referencia de tarifas existente (VALOR ESCALA MENSAJES.docx) para tono/formato
- [x] QA en vivo de TuBanc (Redis + YCloud) — completado, todo saludable
- [x] Limpiar archivos huérfanos de routers/maipa/ (9 archivos, verificados repo-wide)
- [x] Auditar TODO el repo en busca de archivos huérfanos (tubanc, fintech, comercial, licitaciones, pymes, raíz)
- [x] Limpiar residuos aprobados de la raíz (check_redis x2 + 4 .txt)
- [x] Investigar bug real de PyMES reportado (capturas: cashflow se queda pegado, ignora PDF)
- [x] Confirmar causa raíz A: response_schema nunca se aplicaba de verdad en pymes_gemini.py (probado en vivo contra Gemini)
- [x] Arreglar pymes_gemini.py para que sí aplique un schema real cuando se le pasa uno
- [x] Crear schemas Pydantic para cashflow/sales/inventory/canvas y conectarlos en cada agente
- [x] Encontrar y arreglar bug crítico adicional: f-string sin escapar en cashflow_agent.py causaba crash en cada mensaje
- [x] Arreglar Bug B: cashflow/inventory/sales ahora extraen el contenido real de PDFs/imágenes subidos
- [x] Arreglar modelo Gemini descontinuado (gemini-1.5-flash) en pymes/config.py y maipa/config.py
- [x] Auditor independiente (CommandCode, deepseek-v4-flash) revisa todo el paquete de cambios
- [x] Fase 1: Ventas descuenta automáticamente del inventario (apply_inventory_movement)
- [x] Fase 2: Reconciliación Redis↔Postgres si expira el snapshot de inventario
- [x] Fase 3: Memoria de empresa compartida (company_shared_memory) en los 4 agentes guiados
- [x] Fase 4a: Rentabilidad por producto (reporte cruzando ventas+inventario)
- [x] Fase 4b: Nuevo flujo guiado Punto de Equilibrio (break-even)
- [x] Backend: abstracción de proveedor de correo + endpoint de envío masivo en routers/comercial (Gmail/n8n ahora, listo para Resend después)
- [x] Backend: plantilla HTML de correo con system design robusto (basada en tablas, a prueba de Outlook/Gmail/Apple Mail)
- [x] Backend: proveedor de IA configurable (Redis) + fallback a Gemini compartido
- [x] Backend: endpoint de borrador de correo asistido por IA (/broadcast/draft)
- [x] Backend: wiring de infografias.py al proveedor configurable
- [x] client-chime-ai: compositor con campos de diseño (título, CTA, color) + botón 'Diseñar con IA'
- [x] client-chime-ai: página/sección de configuración del proveedor de IA (admin)
- [x] Diseñar el patrón común: identidad (teléfono>BSUID), campo destinatario de salida, registro Redis con reverse-lookup bsuid→teléfono
- [x] Investigar el parsing inbound/outbound actual de comercial, maipa, pymes e ITR-AGENT
- [x] Aplicar el arreglo en ITR-AGENT (el más pequeño/nuevo)
- [x] Aplicar el arreglo en comercial
- [x] Aplicar el arreglo en maipa
- [x] Aplicar el arreglo en pymes
- [x] Diagnosticar por qué YCloud no responde a contactos con teléfono oculto (Sant gallego)
- [x] Investigar documentación oficial de YCloud sobre BSUID (webhook + envío)
- [x] Arreglar routers/tubanc/ycloud.py: campo destinatario (to/recipient) en todos los builders
- [x] Arreglar routers/tubanc/memory.py y routers/fintech/memory.py: session_id + resolve_identity + registro

### In Progress

- [-] Investigar a fondo el bug de la máquina de estados de maipa (comparar con pymes/flow_engine.py que sí funciona)
- [-] Fase 1 (crítico): SSRF en licitaciones + clasificador clínico muerto en maipa + rol en comercial + auth en mark-handled
- [-] Fase 2 (alto): máquina de estados de maipa + XSS licitaciones + dedup atómico fintech
- [-] Fase 3 (medio): regresión pymes cashflow/inventario/ventas + historial duplicado + logs Supabase + otros
- [-] Auditor independiente de opencode (sin contexto) revisa todos los diffs antes de commitear
- [-] Auditor independiente de opencode revisa los 3 fixes antes de dar por terminado
- [-] Redactar contenido completo de la propuesta (3 soluciones + info adicional solicitada)
- [-] Auditar TODO el repo (tubanc, fintech, comercial, licitaciones, pymes, raíz) en busca de archivos huérfanos
- [-] Analizar logs de HOY (12 ago) inbound/outbound para confirmar que los fixes de TuBanc funcionan
- [-] Crear schemas Pydantic para cashflow/sales/inventory/canvas y conectarlos en cada agente
- [-] Fase 1: Ventas descuenta automáticamente del inventario (apply_inventory_movement)
- [-] Fase 2: Reconciliación Redis↔Postgres si expira el snapshot de inventario
- [-] Fase 3: Memoria de empresa compartida (company_shared_memory) en los 4 agentes guiados
- [-] Fase 4: Diseñar y construir plantilla Punto de Equilibrio (break-even)
- [-] Fase 4b: Nuevo flujo guiado Punto de Equilibrio (break-even)
- [-] Fase 4c: Nuevo flujo guiado Cotización/Presupuesto para clientes
- [-] Backend: abstracción de proveedor de correo + endpoint de envío masivo en routers/comercial (Gmail/n8n ahora, listo para Resend después)
- [-] Dar instrucciones exactas de n8n (Reply-To en el nodo Gmail, confirmar Bcc, confirmar carpeta Drive pública)
- [-] client-chime-ai: compositor con campos de diseño (título, CTA, color) + botón 'Diseñar con IA'
- [-] Diseñar el patrón común: identidad (teléfono>BSUID), campo destinatario de salida, registro Redis con reverse-lookup bsuid→teléfono
- [-] Aplicar el arreglo en comercial
- [-] Aplicar el arreglo en maipa
- [-] Aplicar el arreglo en pymes
- [-] Arreglar routers/fintech/router.py (beto_webhook, el webhook compartido): resolver identidad con fromUserId + get_contact_phone_by_id

### Pending

- [ ] Investigar esquema de roles en maipa/comercial para el fix de autorización
- [ ] Fase 1 (crítico): SSRF en licitaciones + clasificador clínico muerto en maipa + rol en comercial + auth en mark-handled
- [ ] Fase 2 (alto): máquina de estados de maipa + XSS/auth licitaciones + dedup atómico fintech
- [ ] Fase 3 (medio): regresión pymes cashflow/inventario/ventas + historial duplicado + logs Supabase + otros
- [ ] Fase 4: unificar la lógica triplicada de 'Bloqueo' entre worker.py/main.py/fintech
- [ ] Auditar todo, verificar que nada se rompió, commit y push
- [ ] Commit y push si el auditor no encuentra problemas
- [ ] Auditar con opencode el código muerto (worker.py, main.py inline worker, ingress.py) y armar un plan de qué hacer con él
- [ ] Commit y push (pendiente confirmación del usuario)
- [ ] Diseñar tablas de precios (mensajería ajustada por Meta, minutos de voz con margen 50%, gestión documental)
- [ ] Construir el .docx con python-docx aplicando paleta y tipografía de TuConsejería
- [ ] Revisar que no se revele stack técnico (FastAPI/Zadarma/ElevenLabs) en el documento final
- [ ] Verificar cada candidato con grep repo-wide antes de borrar (lección aprendida con qdrat_client.py)
- [ ] Confirmar con el usuario y borrar lo confirmado, verificando import completo de la app después
- [ ] Arreglar Bug B: cashflow/inventory/sales ignoran el contenido real de PDFs subidos (solo reciben el marcador [DOC:...])
- [ ] Verificar cada fix (compilar, pyflakes, import completo) y auditor independiente con opencode/CommandCode (deepseek-v4-flash)
- [ ] Fase 2: Reconciliación Redis↔Postgres si expira el snapshot de inventario
- [ ] Fase 3: Memoria de empresa compartida (company_shared_memory) en los 4 agentes guiados
- [ ] Fase 4: Plantillas nuevas (punto de equilibrio, cotización, rentabilidad por producto)
- [ ] Fase 4: Diseñar y construir plantilla Cotización/Presupuesto para clientes
- [ ] Fase 4: Diseñar y construir plantilla Rentabilidad por producto
- [ ] Fase 4c: Nuevo flujo guiado Cotización/Presupuesto para clientes
- [ ] Backend: plantilla HTML de correo con system design robusto (basada en tablas, a prueba de Outlook/Gmail/Apple Mail)
- [ ] Dar instrucciones exactas de n8n (Reply-To en el nodo Gmail, confirmar Bcc, confirmar carpeta Drive pública)
- [ ] client-chime-ai: UI de composición de correo masivo (selector de destinatarios, adjuntos vía Drive, preview, envío) — editar y pushear directo
- [ ] Infografías: vendorizar el motor actual (~/.claude/skills/infografias/) dentro de routers/comercial como herramienta reutilizable
- [ ] client-chime-ai: página/sección de configuración del proveedor de IA (admin)
- [ ] Investigar el parsing inbound/outbound actual de comercial, maipa, pymes e ITR-AGENT
- [ ] Aplicar el arreglo en ITR-AGENT (el más pequeño/nuevo)
- [ ] Aplicar el arreglo en comercial
- [ ] Aplicar el arreglo en maipa
- [ ] Aplicar el arreglo en pymes
- [ ] Verificar de punta a punta con un payload real de contacto sin teléfono

