# ADR 0025 · Fase 7a: detectar un motor que se ha cerrado solo

- **Estado:** aceptada
- **Fecha:** 2026-11-11

## Problema

El planificador guardaba el motor cargado y, si se volvía a pedir el mismo modelo, lo prestaba **sin comprobar que
su proceso seguía vivo**. Si `llama-server` se cerraba solo (falta de memoria, un fallo, alguien lo terminaba), cada
mensaje fallaba hasta que el modelo se descargaba por inactividad (5 minutos por defecto) y el estado seguía diciendo
«Motor listo». Existía `Motor::sigue_vivo`, pero necesitaba `&mut` (`Child::try_wait`) y el planificador comparte el
motor entre hilos, así que nunca se usaba. Se encontró leyendo el código en la revisión de la Fase 7.

## Decisión

- `locall_system::proceso_vivo(&Child)`: pregunta a Windows con `WaitForSingleObject(proceso, 0)` (espera de 0 ms:
  solo «¿has terminado?»). No necesita `&mut`, así que sirve con el motor compartido. `Motor::sigue_vivo` pasa a
  `&self`.
- El trait `Arrancador` gana `sigue_vivo(&motor)` (por defecto, siempre vivo); `ArrancadorReal` lo implementa y, si el
  motor ha muerto, apunta en el registro la causa que dejó el motor (`Motor::causa_del_cierre`).
- El planificador:
  - **al prestar** el modelo que ya está cargado, comprueba que vive; si no, lo retira y lo carga de nuevo (si alguien
    lo tiene prestado, espera a que lo suelte: su petición fallará por su cuenta);
  - **el vigilante**, mientras hay un modelo cargado que nadie usa, comprueba cada 2 s que el motor vive y, si no, lo
    retira: el estado pasa a «Ningún modelo cargado» en vez de mentir.

## Pruebas

- Con un motor de imitación que la prueba «mata»: se recarga al pedirlo, el vigilante lo retira solo, y no se le
  quita a quien lo tiene prestado.
- Con el motor real: la prueba termina el proceso que ella misma arrancó; Locall lo retira en menos de 2 s, carga uno
  nuevo y responde.
