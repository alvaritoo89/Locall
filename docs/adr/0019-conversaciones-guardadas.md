# ADR 0019 · Conversaciones guardadas: en el núcleo, borrado real y un mensaje cada vez

- **Estado:** aceptada
- **Fecha:** 2026-11-09

## Decisión

- **Las conversaciones las guarda el núcleo**, en el mismo registro SQLite que los modelos (esquema v2: tablas
  `conversaciones` y `mensajes`, con borrado en cascada). La app y cualquier otro cliente ven las mismas.
- **Se guardan solo en tu equipo** (`%LOCALAPPDATA%\Locall\locall.db`), nunca salen de él (ADR 0008) y no
  aparecen en los registros diarios (ADR 0014).
- **Borrar es borrar de verdad.** SQLite, al borrar, no sobrescribe los datos: el texto seguiría en el archivo y en su
  registro de cambios (`-wal`). Por eso la conexión usa `PRAGMA secure_delete=ON` (rellena con ceros lo borrado) y, al
  eliminar una conversación, se vacía el `-wal` (`wal_checkpoint(TRUNCATE)`). Hay una prueba que busca el texto en
  los archivos antes y después de borrar.
- **Locall envía al modelo la conversación guardada entera** (con las instrucciones del sistema delante): el cliente
  solo manda el mensaje nuevo (`POST /api/conversations/{id}/messages`). Los ajustes (modelo, instrucciones,
  temperatura) se guardan con la conversación en cada mensaje.
- **Nada a medias:**
  - si el modelo falla (por ejemplo, la conversación ya no cabe), la pregunta no se queda guardada;
  - si se para antes de que llegue nada, tampoco;
  - si se para a mitad, se guarda lo que llegó, marcado como cortado.
- **Cada conversación atiende un mensaje cada vez.** Si llega otro mientras la anterior aún se guarda (por ejemplo,
  al parar y enviar enseguida), espera su turno, para que los mensajes no se guarden desordenados.
- **Se comprueba antes de guardar** que existen la conversación y el modelo (en ese orden).
- La app crea la conversación **al enviar el primer mensaje** (con su principio como título), para no dejar
  conversaciones vacías.

## Rutas

| Ruta | Qué hace |
|------|----------|
| `GET /api/conversations` | Lista (de la más reciente a la más antigua) |
| `POST /api/conversations` | Crea una vacía (`{"title"?}`) |
| `GET /api/conversations/{id}` | Una conversación con sus mensajes |
| `PATCH /api/conversations/{id}` | Cambia el título (`{"title"}`) |
| `DELETE /api/conversations/{id}` | La elimina de verdad |
| `POST /api/conversations/{id}/messages` | Escribe un mensaje (`{"content", "model", "system"?, "temperature"?, "stream"?}`) y responde en NDJSON; el último evento lleva la conversación y el mensaje guardados |

Una conversación que no existe da `404 conversation_not_found`.

## Motivos

- **En el núcleo y no en la app:** así la CLI podrá abrir las mismas conversaciones más adelante, y la app sigue
  siendo solo una interfaz (regla 9).
- **Borrado real:** la privacidad local es un principio de diseño (regla 18). Quien borra una conversación espera que
  no se pueda recuperar del disco.
- **La conversación entera la monta el núcleo:** lo que se guarda y lo que ve el modelo no pueden diferir.

## Consecuencias

- `secure_delete` hace los borrados algo más lentos (escribe ceros). Con conversaciones de texto no se nota.
- Las conversaciones **no se cifran** en el disco: las protege la cuenta de Windows, como el resto de tus archivos.
  Cifrarlas se valorará en la Fase 7 (endurecimiento).
- Si haces copias de seguridad que incluyan `%LOCALAPPDATA%\Locall`, esas copias pueden conservar conversaciones ya
  borradas. Locall no puede evitarlo.
