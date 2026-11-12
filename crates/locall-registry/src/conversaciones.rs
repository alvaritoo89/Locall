//! Saved conversations (schema v2; encrypted since v3). They live in the core so the app and the CLI share them
//! (decision confirmed during research).
//!
//! **Encrypted (schema v3, ADR 0030):** titles, system instructions, message texts and reasoning are stored
//! encrypted with the user's Windows key (DPAPI, [`locall_system::proteger`]). Dates, figures, the model name and the
//! temperature are not: they reveal nothing about what is said, and they are needed to sort and list without
//! decrypting. An empty text is stored as an empty value (there is nothing to hide in it).
//!
//! **Real deletion (privacy):** when deleting, SQLite does not overwrite the data: the text would stay in the file
//! until that space is reused, and in the change log (`-wal`). That is why the connection uses `secure_delete` (fills
//! deleted data with zeros) and, when a conversation is deleted, the `-wal` is emptied (`wal_checkpoint(TRUNCATE)`).

use rusqlite::{OptionalExtension, Row, Transaction, params};

use crate::{ErrorRegistro, Registro, Resultado, a_i64, a_u64, invalido};

/// DPAPI "purpose" of the conversation texts: decrypting them requires the same one (ADR 0030).
const PROPOSITO: &[u8] = b"Locall: conversaciones v1";

/// What is shown instead of a text that cannot be decrypted (saved by another Windows user, on another computer, or
/// after the user's key was lost in a forced password reset).
pub const NO_DESCIFRABLE: &str = "⚠ No se puede descifrar este texto: se guardó con otro usuario de Windows, en otro \
                                  equipo o antes de un cambio forzado de la contraseña de Windows.";

/// Encrypts a text to store it. An empty text stays empty.
pub(crate) fn cifrar(texto: &str) -> Resultado<Vec<u8>> {
    if texto.is_empty() {
        return Ok(Vec::new());
    }
    locall_system::proteger(texto.as_bytes(), PROPOSITO).map_err(ErrorRegistro::Cifrado)
}

/// Decrypts a stored text. It never fails: what cannot be decrypted becomes [`NO_DESCIFRABLE`], so one unreadable
/// message does not make the whole conversation (or the list) unusable.
pub(crate) fn descifrar(cifrado: &[u8]) -> String {
    if cifrado.is_empty() {
        return String::new();
    }
    locall_system::desproteger(cifrado, PROPOSITO)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or_else(|| NO_DESCIFRABLE.to_owned())
}

/// Migration to schema version 3 (ADR 0030): the tables are rebuilt with the texts as encrypted `BLOB`s (the old
/// `STRICT` tables only accept text there) and every existing conversation is encrypted, all inside the migration's
/// transaction. Ids, order and everything else are kept as they were.
pub(crate) fn cifrar_conversaciones(tx: &Transaction<'_>) -> Resultado<()> {
    tx.execute_batch(
        "
        ALTER TABLE mensajes RENAME TO mensajes_v2;
        ALTER TABLE conversaciones RENAME TO conversaciones_v2;
        DROP INDEX conversaciones_por_fecha;

        CREATE TABLE conversaciones (
            id           INTEGER PRIMARY KEY,
            titulo       BLOB    NOT NULL,
            modelo       TEXT,
            sistema      BLOB    NOT NULL DEFAULT X'',
            temperatura  REAL,
            creada       INTEGER NOT NULL,
            actualizada  INTEGER NOT NULL
        ) STRICT;

        CREATE TABLE mensajes (
            conversacion_id  INTEGER NOT NULL REFERENCES conversaciones(id) ON DELETE CASCADE,
            orden            INTEGER NOT NULL,
            rol              TEXT    NOT NULL CHECK (rol IN ('user', 'assistant')),
            contenido        BLOB    NOT NULL,
            razonamiento     BLOB    NOT NULL DEFAULT X'',
            modelo           TEXT,
            tokens           INTEGER,
            velocidad        REAL,
            cortada          INTEGER NOT NULL DEFAULT 0 CHECK (cortada IN (0, 1)),
            creado           INTEGER NOT NULL,
            PRIMARY KEY (conversacion_id, orden)
        ) STRICT;
        ",
    )?;
    {
        let mut leer = tx.prepare(
            "SELECT id, titulo, modelo, sistema, temperatura, creada, actualizada FROM conversaciones_v2",
        )?;
        let mut filas = leer.query([])?;
        while let Some(f) = filas.next()? {
            tx.execute(
                "INSERT INTO conversaciones (id, titulo, modelo, sistema, temperatura, creada, actualizada)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    f.get::<_, i64>(0)?,
                    cifrar(&f.get::<_, String>(1)?)?,
                    f.get::<_, Option<String>>(2)?,
                    cifrar(&f.get::<_, String>(3)?)?,
                    f.get::<_, Option<f64>>(4)?,
                    f.get::<_, i64>(5)?,
                    f.get::<_, i64>(6)?,
                ],
            )?;
        }
    }
    {
        let mut leer = tx.prepare(
            "SELECT conversacion_id, orden, rol, contenido, razonamiento, modelo, tokens, velocidad, cortada, creado
             FROM mensajes_v2",
        )?;
        let mut filas = leer.query([])?;
        while let Some(f) = filas.next()? {
            tx.execute(
                "INSERT INTO mensajes (conversacion_id, orden, rol, contenido, razonamiento, modelo, tokens, velocidad,
                                       cortada, creado)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    f.get::<_, i64>(0)?,
                    f.get::<_, i64>(1)?,
                    f.get::<_, String>(2)?,
                    cifrar(&f.get::<_, String>(3)?)?,
                    cifrar(&f.get::<_, String>(4)?)?,
                    f.get::<_, Option<String>>(5)?,
                    f.get::<_, Option<i64>>(6)?,
                    f.get::<_, Option<f64>>(7)?,
                    f.get::<_, i64>(8)?,
                    f.get::<_, i64>(9)?,
                ],
            )?;
        }
    }
    tx.execute_batch(
        "
        DROP TABLE mensajes_v2;
        DROP TABLE conversaciones_v2;
        CREATE INDEX conversaciones_por_fecha ON conversaciones(actualizada);
        ",
    )?;
    Ok(())
}

/// Maximum length of a title (in characters).
pub const TITULO_MAX: usize = 120;

/// Who wrote a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Autor {
    Usuario,
    Asistente,
}

impl Autor {
    fn como_texto(self) -> &'static str {
        match self {
            Autor::Usuario => "user",
            Autor::Asistente => "assistant",
        }
    }

    fn desde_texto(texto: &str) -> Resultado<Autor> {
        match texto {
            "user" => Ok(Autor::Usuario),
            "assistant" => Ok(Autor::Asistente),
            otro => Err(invalido("mensajes.rol", otro)),
        }
    }
}

/// A conversation (without its messages).
#[derive(Debug, Clone, PartialEq)]
pub struct Conversacion {
    pub id: i64,
    pub titulo: String,
    /// The last model used (it may have been removed since).
    pub modelo: Option<String>,
    /// System instructions.
    pub sistema: String,
    pub temperatura: Option<f64>,
    pub creada: u64,
    pub actualizada: u64,
    pub mensajes: u64,
}

/// A saved message.
#[derive(Debug, Clone, PartialEq)]
pub struct MensajeGuardado {
    pub orden: u64,
    pub autor: Autor,
    pub contenido: String,
    pub razonamiento: String,
    /// In answers: the model that answered and its figures.
    pub modelo: Option<String>,
    pub tokens: Option<u64>,
    pub velocidad: Option<f64>,
    /// The answer was stopped before it finished.
    pub cortada: bool,
    pub creado: u64,
}

/// The data of a new message.
#[derive(Debug, Clone, PartialEq)]
pub struct MensajeNuevo {
    pub autor: Autor,
    pub contenido: String,
    pub razonamiento: String,
    pub modelo: Option<String>,
    pub tokens: Option<u64>,
    pub velocidad: Option<f64>,
    pub cortada: bool,
}

impl MensajeNuevo {
    /// A user message.
    pub fn del_usuario(contenido: impl Into<String>) -> MensajeNuevo {
        MensajeNuevo {
            autor: Autor::Usuario,
            contenido: contenido.into(),
            razonamiento: String::new(),
            modelo: None,
            tokens: None,
            velocidad: None,
            cortada: false,
        }
    }
}

/// Trims a title to [`TITULO_MAX`] characters, on a single line.
pub fn titulo_valido(texto: &str) -> String {
    let linea = texto.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut titulo: String = linea.chars().take(TITULO_MAX).collect();
    if linea.chars().count() > TITULO_MAX {
        titulo.push('…');
    }
    titulo
}

const COLUMNAS_CONVERSACION: &str =
    "c.id, c.titulo, c.modelo, c.sistema, c.temperatura, c.creada, c.actualizada,
     (SELECT count(*) FROM mensajes m WHERE m.conversacion_id = c.id)";

/// A conversation row as it comes out of SQLite (id, title, model, system, temperature, dates, messages).
type Fila = (
    i64,
    Vec<u8>,
    Option<String>,
    Vec<u8>,
    Option<f64>,
    i64,
    i64,
    i64,
);

fn conversacion_de(f: &Row<'_>) -> rusqlite::Result<Fila> {
    Ok((
        f.get(0)?,
        f.get(1)?,
        f.get(2)?,
        f.get(3)?,
        f.get(4)?,
        f.get(5)?,
        f.get(6)?,
        f.get(7)?,
    ))
}

fn convertir(
    (id, titulo, modelo, sistema, temperatura, creada, actualizada, mensajes): Fila,
) -> Resultado<Conversacion> {
    Ok(Conversacion {
        id,
        titulo: descifrar(&titulo),
        modelo,
        sistema: descifrar(&sistema),
        temperatura,
        creada: a_u64(creada, "conversaciones.creada")?,
        actualizada: a_u64(actualizada, "conversaciones.actualizada")?,
        mensajes: a_u64(mensajes, "mensajes")?,
    })
}

fn no_encontrada(id: i64) -> ErrorRegistro {
    ErrorRegistro::NoEncontrado(format!("conversación {id}"))
}

impl Registro {
    /// Creates an empty conversation.
    pub fn crear_conversacion(&mut self, titulo: &str, ahora: u64) -> Resultado<Conversacion> {
        let momento = a_i64(ahora, "conversaciones.creada")?;
        self.conexion.execute(
            "INSERT INTO conversaciones (titulo, creada, actualizada) VALUES (?1, ?2, ?2)",
            params![cifrar(&titulo_valido(titulo))?, momento],
        )?;
        let id = self.conexion.last_insert_rowid();
        self.conversacion(id)?.ok_or_else(|| no_encontrada(id))
    }

    /// The conversations, from the most recent to the oldest.
    pub fn conversaciones(&self) -> Resultado<Vec<Conversacion>> {
        let mut consulta = self.conexion.prepare(&format!(
            "SELECT {COLUMNAS_CONVERSACION} FROM conversaciones c ORDER BY c.actualizada DESC, c.id DESC"
        ))?;
        let filas = consulta.query_map([], conversacion_de)?;
        filas.map(|f| convertir(f?)).collect()
    }

    /// A conversation (without its messages).
    pub fn conversacion(&self, id: i64) -> Resultado<Option<Conversacion>> {
        let fila = self
            .conexion
            .query_row(
                &format!("SELECT {COLUMNAS_CONVERSACION} FROM conversaciones c WHERE c.id = ?1"),
                [id],
                conversacion_de,
            )
            .optional()?;
        fila.map(convertir).transpose()
    }

    /// The messages of a conversation, in order.
    pub fn mensajes(&self, id: i64) -> Resultado<Vec<MensajeGuardado>> {
        let mut consulta = self.conexion.prepare(
            "SELECT orden, rol, contenido, razonamiento, modelo, tokens, velocidad, cortada, creado
             FROM mensajes WHERE conversacion_id = ?1 ORDER BY orden",
        )?;
        let filas = consulta.query_map([id], |f| {
            Ok((
                f.get::<_, i64>(0)?,
                f.get::<_, String>(1)?,
                f.get::<_, Vec<u8>>(2)?,
                f.get::<_, Vec<u8>>(3)?,
                f.get::<_, Option<String>>(4)?,
                f.get::<_, Option<i64>>(5)?,
                f.get::<_, Option<f64>>(6)?,
                f.get::<_, i64>(7)?,
                f.get::<_, i64>(8)?,
            ))
        })?;
        filas
            .map(|fila| {
                let (
                    orden,
                    rol,
                    contenido,
                    razonamiento,
                    modelo,
                    tokens,
                    velocidad,
                    cortada,
                    creado,
                ) = fila?;
                Ok(MensajeGuardado {
                    orden: a_u64(orden, "mensajes.orden")?,
                    autor: Autor::desde_texto(&rol)?,
                    contenido: descifrar(&contenido),
                    razonamiento: descifrar(&razonamiento),
                    modelo,
                    tokens: tokens.map(|t| a_u64(t, "mensajes.tokens")).transpose()?,
                    velocidad,
                    cortada: cortada != 0,
                    creado: a_u64(creado, "mensajes.creado")?,
                })
            })
            .collect()
    }

    /// Changes the title.
    pub fn renombrar_conversacion(&mut self, id: i64, titulo: &str) -> Resultado<()> {
        let cambiadas = self.conexion.execute(
            "UPDATE conversaciones SET titulo = ?2 WHERE id = ?1",
            params![id, cifrar(&titulo_valido(titulo))?],
        )?;
        if cambiadas == 0 {
            return Err(no_encontrada(id));
        }
        Ok(())
    }

    /// Saves the model, system instructions and temperature the conversation is being used with.
    pub fn ajustar_conversacion(
        &mut self,
        id: i64,
        modelo: &str,
        sistema: &str,
        temperatura: Option<f64>,
    ) -> Resultado<()> {
        let cambiadas = self.conexion.execute(
            "UPDATE conversaciones SET modelo = ?2, sistema = ?3, temperatura = ?4 WHERE id = ?1",
            params![id, modelo, cifrar(sistema)?, temperatura],
        )?;
        if cambiadas == 0 {
            return Err(no_encontrada(id));
        }
        Ok(())
    }

    /// Appends a message at the end and marks the conversation as updated. Returns the saved message.
    pub fn anadir_mensaje(
        &mut self,
        id: i64,
        m: &MensajeNuevo,
        ahora: u64,
    ) -> Resultado<MensajeGuardado> {
        let momento = a_i64(ahora, "mensajes.creado")?;
        let tx = self.conexion.transaction()?;
        let existe: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM conversaciones WHERE id = ?1)",
            [id],
            |f| f.get(0),
        )?;
        if !existe {
            return Err(no_encontrada(id));
        }
        let orden: i64 = tx.query_row(
            "SELECT coalesce(max(orden), -1) + 1 FROM mensajes WHERE conversacion_id = ?1",
            [id],
            |f| f.get(0),
        )?;
        let tokens = m.tokens.map(|t| a_i64(t, "mensajes.tokens")).transpose()?;
        let (contenido, razonamiento) = (cifrar(&m.contenido)?, cifrar(&m.razonamiento)?);
        tx.execute(
            "INSERT INTO mensajes (conversacion_id, orden, rol, contenido, razonamiento, modelo, tokens, velocidad,
                                   cortada, creado)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                id,
                orden,
                m.autor.como_texto(),
                contenido,
                razonamiento,
                m.modelo,
                tokens,
                m.velocidad,
                i64::from(m.cortada),
                momento
            ],
        )?;
        tx.execute(
            "UPDATE conversaciones SET actualizada = ?2 WHERE id = ?1",
            params![id, momento],
        )?;
        tx.commit()?;
        Ok(MensajeGuardado {
            orden: a_u64(orden, "mensajes.orden")?,
            autor: m.autor,
            contenido: m.contenido.clone(),
            razonamiento: m.razonamiento.clone(),
            modelo: m.modelo.clone(),
            tokens: m.tokens,
            velocidad: m.velocidad,
            cortada: m.cortada,
            creado: ahora,
        })
    }

    /// Removes the last message (for example, a question that got no answer because of an error).
    pub fn quitar_ultimo_mensaje(&mut self, id: i64) -> Resultado<()> {
        self.conexion.execute(
            "DELETE FROM mensajes WHERE conversacion_id = ?1
               AND orden = (SELECT max(orden) FROM mensajes WHERE conversacion_id = ?1)",
            [id],
        )?;
        Ok(())
    }

    /// Deletes a conversation and its messages **for real** (see the top of this module).
    pub fn eliminar_conversacion(&mut self, id: i64) -> Resultado<()> {
        let borradas = self
            .conexion
            .execute("DELETE FROM conversaciones WHERE id = ?1", [id])?;
        if borradas == 0 {
            return Err(no_encontrada(id));
        }
        self.vaciar_registro_de_cambios()?;
        Ok(())
    }

    /// Moves the change log (`-wal`) into the main file and leaves it empty, so no copy of the deleted data stays there.
    /// (It does nothing on an in-memory database.)
    fn vaciar_registro_de_cambios(&self) -> Resultado<()> {
        self.conexion
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
            .optional()?;
        Ok(())
    }
}
