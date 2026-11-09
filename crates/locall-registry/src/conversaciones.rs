//! Conversaciones guardadas (esquema v2). Viven en el núcleo para que la app y la CLI las compartan (decisión
//! ratificada en la investigación).
//!
//! **Borrado real (privacidad):** SQLite, al borrar, no sobrescribe los datos: el texto se quedaría en el archivo hasta
//! reutilizar ese espacio, y en el registro de cambios (`-wal`). Por eso la conexión usa `secure_delete` (rellena con
//! ceros lo borrado) y, al eliminar una conversación, se vacía el `-wal` (`wal_checkpoint(TRUNCATE)`).

use rusqlite::{OptionalExtension, Row, params};

use crate::{ErrorRegistro, Registro, Resultado, a_i64, a_u64, invalido};

/// Longitud máxima de un título (en caracteres).
pub const TITULO_MAX: usize = 120;

/// Quién escribió un mensaje.
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

/// Una conversación (sin sus mensajes).
#[derive(Debug, Clone, PartialEq)]
pub struct Conversacion {
    pub id: i64,
    pub titulo: String,
    /// El último modelo usado (puede haberse eliminado después).
    pub modelo: Option<String>,
    /// Instrucciones del sistema.
    pub sistema: String,
    pub temperatura: Option<f64>,
    pub creada: u64,
    pub actualizada: u64,
    pub mensajes: u64,
}

/// Un mensaje guardado.
#[derive(Debug, Clone, PartialEq)]
pub struct MensajeGuardado {
    pub orden: u64,
    pub autor: Autor,
    pub contenido: String,
    pub razonamiento: String,
    /// En las respuestas: el modelo que respondió y sus cifras.
    pub modelo: Option<String>,
    pub tokens: Option<u64>,
    pub velocidad: Option<f64>,
    /// La respuesta se paró antes de terminar.
    pub cortada: bool,
    pub creado: u64,
}

/// Los datos de un mensaje nuevo.
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
    /// Un mensaje del usuario.
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

/// Recorta un título a [`TITULO_MAX`] caracteres, en una sola línea.
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

/// Una fila de conversación tal como sale de SQLite (id, título, modelo, sistema, temperatura, fechas, mensajes).
type Fila = (
    i64,
    String,
    Option<String>,
    String,
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
        titulo,
        modelo,
        sistema,
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
    /// Crea una conversación vacía.
    pub fn crear_conversacion(&mut self, titulo: &str, ahora: u64) -> Resultado<Conversacion> {
        let momento = a_i64(ahora, "conversaciones.creada")?;
        self.conexion.execute(
            "INSERT INTO conversaciones (titulo, creada, actualizada) VALUES (?1, ?2, ?2)",
            params![titulo_valido(titulo), momento],
        )?;
        let id = self.conexion.last_insert_rowid();
        self.conversacion(id)?.ok_or_else(|| no_encontrada(id))
    }

    /// Las conversaciones, de la más reciente a la más antigua.
    pub fn conversaciones(&self) -> Resultado<Vec<Conversacion>> {
        let mut consulta = self.conexion.prepare(&format!(
            "SELECT {COLUMNAS_CONVERSACION} FROM conversaciones c ORDER BY c.actualizada DESC, c.id DESC"
        ))?;
        let filas = consulta.query_map([], conversacion_de)?;
        filas.map(|f| convertir(f?)).collect()
    }

    /// Una conversación (sin sus mensajes).
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

    /// Los mensajes de una conversación, en orden.
    pub fn mensajes(&self, id: i64) -> Resultado<Vec<MensajeGuardado>> {
        let mut consulta = self.conexion.prepare(
            "SELECT orden, rol, contenido, razonamiento, modelo, tokens, velocidad, cortada, creado
             FROM mensajes WHERE conversacion_id = ?1 ORDER BY orden",
        )?;
        let filas = consulta.query_map([id], |f| {
            Ok((
                f.get::<_, i64>(0)?,
                f.get::<_, String>(1)?,
                f.get::<_, String>(2)?,
                f.get::<_, String>(3)?,
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
                    contenido,
                    razonamiento,
                    modelo,
                    tokens: tokens.map(|t| a_u64(t, "mensajes.tokens")).transpose()?,
                    velocidad,
                    cortada: cortada != 0,
                    creado: a_u64(creado, "mensajes.creado")?,
                })
            })
            .collect()
    }

    /// Cambia el título.
    pub fn renombrar_conversacion(&mut self, id: i64, titulo: &str) -> Resultado<()> {
        let cambiadas = self.conexion.execute(
            "UPDATE conversaciones SET titulo = ?2 WHERE id = ?1",
            params![id, titulo_valido(titulo)],
        )?;
        if cambiadas == 0 {
            return Err(no_encontrada(id));
        }
        Ok(())
    }

    /// Guarda el modelo, las instrucciones del sistema y la temperatura con que se está usando la conversación.
    pub fn ajustar_conversacion(
        &mut self,
        id: i64,
        modelo: &str,
        sistema: &str,
        temperatura: Option<f64>,
    ) -> Resultado<()> {
        let cambiadas = self.conexion.execute(
            "UPDATE conversaciones SET modelo = ?2, sistema = ?3, temperatura = ?4 WHERE id = ?1",
            params![id, modelo, sistema, temperatura],
        )?;
        if cambiadas == 0 {
            return Err(no_encontrada(id));
        }
        Ok(())
    }

    /// Añade un mensaje al final y marca la conversación como actualizada. Devuelve el mensaje guardado.
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
        tx.execute(
            "INSERT INTO mensajes (conversacion_id, orden, rol, contenido, razonamiento, modelo, tokens, velocidad,
                                   cortada, creado)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                id,
                orden,
                m.autor.como_texto(),
                m.contenido,
                m.razonamiento,
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

    /// Quita el último mensaje (por ejemplo, una pregunta que no obtuvo respuesta por un error).
    pub fn quitar_ultimo_mensaje(&mut self, id: i64) -> Resultado<()> {
        self.conexion.execute(
            "DELETE FROM mensajes WHERE conversacion_id = ?1
               AND orden = (SELECT max(orden) FROM mensajes WHERE conversacion_id = ?1)",
            [id],
        )?;
        Ok(())
    }

    /// Elimina una conversación y sus mensajes **de verdad** (ver el inicio de este módulo).
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

    /// Pasa el registro de cambios (`-wal`) al archivo principal y lo deja vacío, para que no quede ahí una copia de
    /// lo borrado. (En una base de datos en memoria no hace nada.)
    fn vaciar_registro_de_cambios(&self) -> Resultado<()> {
        self.conexion
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
            .optional()?;
        Ok(())
    }
}
