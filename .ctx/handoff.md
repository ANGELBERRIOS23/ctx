# Handoff: ctx

- **Created:** 2026-09-04T21:31:16.506+00:00
- **Source Agent:** claude
- **Source Machine:** Mac-mini.local
- **Git Branch:** HEAD

## Summary

Ahora sí, comparación completa con datos reales:
| | **EC2 `g7e.2xlarge`** | **SageMaker `ml.g6e.2xlarge`** | **Mac M4 Pro** |
|---|---|---|---|
| Hardware | RTX PRO 6000 **96GB** | L40S 48GB | 48GB unificada (~38 usables) |
| Variante que corre | bf16 completo (55.6GB) o FP8 | FP8 (30.9GB) | MLX 8-bit (29.5GB) o 4-bit (16.1GB) |
| Precio | **$3.36/h** on-demand | ~$2.80/h | no confirmado (~$1.5-2/h) |
| Facturación mínima | por segundo, apagás cuando querés | por segundo | **mínimo 24 horas** |
| Ancho de banda de memoria | ~1.8 TB/s | ~864 GB/s | ~273 GB/s |
Tres cosas que definen la decisión:
**1. El spot no te lo aprobaron** (`All G and VT Spot` sigue en 0), así que los $1.60/h que vi en el mercado no los podés usar. Es $3.36/h on-demand o nada.
**2. El mínimo de 24 horas del Mac es el problema real.** Para experimentar —que es lo que vas a hacer— pagar 24h obligatorias cada vez que lo prendés es malísimo. En la GPU prendés 3 horas, gastás $10, y la apagás.
**3. Velocidad: el ancho de banda de memoria es lo que manda** en generación de tokens, y ahí el Mac va ~6× más lento que la RTX PRO 6000. No es un detalle menor cuando estás probando prompts.
**Pero hay un punto genuino a favor del Mac que quiero que sepas:** la conversión MLX ya existe y tiene **134K descargas**, o sea que está probado que funciona en Apple Silicon. En cambio la ruta de vLLM con la arquitectura `Qwen3_5ForConditionalGeneration` sigue sin verificar — es el riesgo que te venía marcando. Si vLLM no la reconoce, en la GPU no arranca y hay que buscar contenedor alternativo.
**Mi recomendación:** `g7e.2xlarge` con la variante **FP8 (30.9GB)**. Te da 96GB para un modelo de 31GB, o sea contexto largo sin apretar, es la más rápida, y sin compromiso de 24 horas. Si vLLM no soporta la arquitectura, el Mac queda como plan de respaldo con la ruta MLX que sí está probada.
Para arrancar sigo necesitando lo mismo: el token de HuggingFace en `~/.hf_token` (el repo está cerrado). Con eso leo el `config.json` real y confirmo el soporte de vLLM **antes** de mover 31GB — así sabemos en cinco minutos si vamos por GPU o por Mac.

primer push exitoso

