use std::path::PathBuf;

use super::*;

fn seg(t: &str) -> Segmento {
    Segmento::nuevo(t).unwrap()
}

fn huella(c: char) -> Huella {
    Huella::desde_hex(&c.to_string().repeat(64)).unwrap()
}

fn archivo(nombre: &str, c: char, bytes: u64) -> ArchivoModelo {
    ArchivoModelo {
        nombre: NombreArchivo::nuevo(nombre).unwrap(),
        huella: huella(c),
        bytes,
    }
}

fn modelo_4b() -> ModeloNuevo {
    ModeloNuevo {
        id: seg("qwen3-4b"),
        nombre: "Qwen3 4B".into(),
        origen: "hf:Qwen/Qwen3-4B-GGUF".into(),
        revision: "bc640142c66e1fdd12af0bd68f40445458f3869b".into(),
        cuantizacion: Some("Q4_K_M".into()),
        licencia: Some("apache-2.0".into()),
        arquitectura: Some("qwen3".into()),
        contexto_max: Some(40_960),
        verificado: true,
        archivos: vec![archivo("Qwen3-4B-Q4_K_M.gguf", 'a', 2_497_280_256)],
    }
}

fn modelo_7b_dividido() -> ModeloNuevo {
    ModeloNuevo {
        id: seg("qwen2.5-7b"),
        nombre: "Qwen2.5 7B".into(),
        origen: "hf:Qwen/Qwen2.5-7B-Instruct-GGUF".into(),
        revision: "bb5d59e06d9551d752d08b292a50eb208b07ab1f".into(),
        cuantizacion: Some("Q4_K_M".into()),
        licencia: Some("apache-2.0".into()),
        arquitectura: Some("qwen2".into()),
        contexto_max: Some(131_072),
        verificado: true,
        // Deliberately out of order.
        archivos: vec![
            archivo(
                "qwen2.5-7b-instruct-q4_k_m-00002-of-00002.gguf",
                'c',
                689_872_288,
            ),
            archivo(
                "qwen2.5-7b-instruct-q4_k_m-00001-of-00002.gguf",
                'b',
                3_993_201_344,
            ),
        ],
    }
}

#[test]
fn registra_y_lee_un_modelo() {
    let mut r = Registro::en_memoria().unwrap();
    r.registrar(&modelo_4b(), 1_000).unwrap();

    let m = r.obtener(&seg("qwen3-4b")).unwrap().unwrap();
    assert_eq!(m.nombre, "Qwen3 4B");
    assert_eq!(m.licencia.as_deref(), Some("apache-2.0"));
    assert_eq!(m.contexto_max, Some(40_960));
    assert!(m.verificado);
    assert_eq!(m.instalado, 1_000);
    assert_eq!(m.ultimo_uso, None);
    assert_eq!(m.bytes_totales(), 2_497_280_256);
    assert_eq!(r.obtener(&seg("no-existe")).unwrap(), None);
}

#[test]
fn ordena_las_partes_de_un_modelo_dividido() {
    let mut r = Registro::en_memoria().unwrap();
    r.registrar(&modelo_7b_dividido(), 1).unwrap();
    let m = r.obtener(&seg("qwen2.5-7b")).unwrap().unwrap();
    assert!(m.archivos[0].nombre.como_str().contains("-00001-of-"));
    assert_eq!(m.bytes_totales(), 3_993_201_344 + 689_872_288);
}

#[test]
fn lista_por_nombre() {
    let mut r = Registro::en_memoria().unwrap();
    r.registrar(&modelo_4b(), 1).unwrap();
    r.registrar(&modelo_7b_dividido(), 1).unwrap();
    let nombres: Vec<String> = r.listar().unwrap().into_iter().map(|m| m.nombre).collect();
    assert_eq!(nombres, ["Qwen2.5 7B", "Qwen3 4B"]);
}

#[test]
fn rechaza_duplicados_y_modelos_sin_archivos() {
    let mut r = Registro::en_memoria().unwrap();
    r.registrar(&modelo_4b(), 1).unwrap();
    assert!(matches!(
        r.registrar(&modelo_4b(), 2),
        Err(ErrorRegistro::YaExiste(_))
    ));
    let mut vacio = modelo_7b_dividido();
    vacio.archivos.clear();
    assert!(matches!(
        r.registrar(&vacio, 1),
        Err(ErrorRegistro::SinArchivos)
    ));
}

#[test]
fn una_transaccion_fallida_no_deja_nada() {
    let mut r = Registro::en_memoria().unwrap();
    let mut roto = modelo_4b();
    // Two files with the same name: the second INSERT fails halfway through the transaction.
    roto.archivos.push(archivo("Qwen3-4B-Q4_K_M.gguf", 'b', 1));
    assert!(r.registrar(&roto, 1).is_err());
    assert_eq!(
        r.obtener(&seg("qwen3-4b")).unwrap(),
        None,
        "no debe quedar el modelo a medias"
    );
    assert!(r.huellas_en_uso().unwrap().is_empty());
}

#[test]
fn marca_el_ultimo_uso() {
    let mut r = Registro::en_memoria().unwrap();
    r.registrar(&modelo_4b(), 1).unwrap();
    r.marcar_uso(&seg("qwen3-4b"), 500).unwrap();
    assert_eq!(
        r.obtener(&seg("qwen3-4b")).unwrap().unwrap().ultimo_uso,
        Some(500)
    );
    assert!(matches!(
        r.marcar_uso(&seg("otro"), 1),
        Err(ErrorRegistro::NoEncontrado(_))
    ));
}

#[test]
fn eliminar_devuelve_las_huellas_y_borra_sus_archivos() {
    let mut r = Registro::en_memoria().unwrap();
    r.registrar(&modelo_4b(), 1).unwrap();
    r.registrar(&modelo_7b_dividido(), 1).unwrap();
    assert_eq!(r.huellas_en_uso().unwrap().len(), 3);

    let huellas = r.eliminar(&seg("qwen2.5-7b")).unwrap();
    assert_eq!(huellas, vec![huella('b'), huella('c')]);
    assert_eq!(r.huellas_en_uso().unwrap(), BTreeSet::from([huella('a')]));
    assert!(matches!(
        r.eliminar(&seg("qwen2.5-7b")),
        Err(ErrorRegistro::NoEncontrado(_))
    ));
}

/// Temporary database file that is deleted at the end (together with the WAL ones, the copies made before migrating and
/// the damaged ones set aside: everything whose name starts with the database's).
struct BaseDeDatosDePrueba(PathBuf);

impl BaseDeDatosDePrueba {
    fn nueva(nombre: &str) -> BaseDeDatosDePrueba {
        let ruta = std::env::temp_dir().join(format!(
            "locall-registro-{nombre}-{}.db",
            std::process::id()
        ));
        let b = BaseDeDatosDePrueba(ruta);
        b.borrar();
        b
    }
    fn borrar(&self) {
        for ruta in self.relacionados() {
            let _ = std::fs::remove_file(ruta);
        }
    }
    /// The files next to the database whose name starts with its own (except the database itself).
    fn relacionados(&self) -> Vec<PathBuf> {
        let nombre = self.0.file_name().unwrap().to_string_lossy().into_owned();
        let mut lista: Vec<PathBuf> = std::fs::read_dir(self.0.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|r| {
                r.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with(&nombre))
            })
            .collect();
        lista.sort();
        lista
    }
    /// The related file whose name contains `marca` (`.danada-`, `.copia-antes-de-`), if there is one.
    fn buscar(&self, marca: &str) -> Option<PathBuf> {
        self.relacionados().into_iter().find(|r| {
            let n = r.file_name().unwrap().to_string_lossy().into_owned();
            n.contains(marca) && !n.ends_with("-wal") && !n.ends_with("-shm")
        })
    }
}

impl Drop for BaseDeDatosDePrueba {
    fn drop(&mut self) {
        self.borrar();
    }
}

#[test]
fn los_datos_persisten_al_cerrar_y_abrir() {
    let db = BaseDeDatosDePrueba::nueva("persiste");
    {
        let mut r = Registro::abrir(&db.0).unwrap();
        assert_eq!(r.version_esquema().unwrap(), VERSION_ESQUEMA);
        r.registrar(&modelo_4b(), 1).unwrap();
    } // it gets closed
    let r = Registro::abrir(&db.0).unwrap(); // opening it again does NOT repeat the migrations
    assert_eq!(r.version_esquema().unwrap(), VERSION_ESQUEMA);
    assert_eq!(r.listar().unwrap().len(), 1);
}

#[test]
fn no_toca_una_base_de_datos_de_una_version_futura() {
    let db = BaseDeDatosDePrueba::nueva("futura");
    {
        let c = Connection::open(&db.0).unwrap();
        c.pragma_update(None, "user_version", 99).unwrap();
    }
    assert!(matches!(
        Registro::abrir(&db.0),
        Err(ErrorRegistro::VersionFutura { encontrada: 99, .. })
    ));
}

#[test]
fn detecta_datos_manipulados() {
    let mut r = Registro::en_memoria().unwrap();
    r.registrar(&modelo_4b(), 1).unwrap();
    // Someone changes the hash by hand to 64 characters that are not hexadecimal.
    r.conexion
        .execute("UPDATE archivos SET huella = ?1", [&"z".repeat(64)])
        .unwrap();
    assert!(matches!(
        r.obtener(&seg("qwen3-4b")),
        Err(ErrorRegistro::DatoInvalido { .. })
    ));
}

// ─── Conversations (schema v2) ────────────────────────────────────────────────

fn respuesta(texto: &str) -> MensajeNuevo {
    MensajeNuevo {
        autor: Autor::Asistente,
        contenido: texto.into(),
        razonamiento: "pensé".into(),
        modelo: Some("qwen3-4b".into()),
        tokens: Some(12),
        velocidad: Some(12.5),
        cortada: false,
    }
}

#[test]
fn una_base_de_datos_v1_se_actualiza_a_v2_sin_perder_nada() {
    let db = BaseDeDatosDePrueba::nueva("v1-a-v2");
    {
        // A database like the Phase 2 one: only migration 1, with one model inside.
        let mut c = Connection::open(&db.0).unwrap();
        let tx = c.transaction().unwrap();
        tx.execute_batch(MIGRACIONES[0]).unwrap();
        tx.execute_batch(
            "INSERT INTO modelos (id, nombre, origen, revision, verificado, instalado)
                 VALUES ('qwen', 'Qwen', 'local', 'local', 0, 1);
             INSERT INTO archivos (modelo_id, nombre, huella, bytes, orden)
                 VALUES ('qwen', 'qwen.gguf', printf('%064d', 0), 10, 0);",
        )
        .unwrap();
        tx.pragma_update(None, "user_version", 1).unwrap();
        tx.commit().unwrap();
    }
    // When it is opened with this version of Locall, migration 2 is applied.
    let mut r = Registro::abrir(&db.0).unwrap();
    assert_eq!(r.version_esquema().unwrap(), 2);
    assert_eq!(r.listar().unwrap().len(), 1, "el modelo sigue ahí");
    assert!(r.conversaciones().unwrap().is_empty());
    r.crear_conversacion("Primera", 5).unwrap();
    assert_eq!(r.conversaciones().unwrap().len(), 1);
    // The copy made before migrating is deleted because everything went well (ADR 0029).
    assert_eq!(db.buscar(".copia-antes-de-"), None);
    assert!(r.base_apartada().is_none());
}

// ─── Integrity and copies (ADR 0029) ──────────────────────────────────────────

#[test]
fn un_archivo_que_no_es_una_base_de_datos_se_aparta_sin_borrarlo() {
    let db = BaseDeDatosDePrueba::nueva("no-es-db");
    let basura = b"esto no es una base de datos de SQLite, pero ocupa algo de sitio".repeat(100);
    std::fs::write(&db.0, &basura).unwrap();

    let r = Registro::abrir(&db.0).unwrap();
    let apartada = r.base_apartada().unwrap().to_path_buf();
    assert!(
        apartada.to_string_lossy().contains(".danada-"),
        "{}",
        apartada.display()
    );
    assert_eq!(
        std::fs::read(&apartada).unwrap(),
        basura,
        "el archivo apartado está intacto"
    );
    assert_eq!(
        r.version_esquema().unwrap(),
        VERSION_ESQUEMA,
        "una base nueva"
    );
    assert!(r.listar().unwrap().is_empty());
}

#[test]
fn una_base_de_datos_estropeada_se_aparta_con_sus_datos() {
    let db = BaseDeDatosDePrueba::nueva("estropeada");
    {
        let mut r = Registro::abrir(&db.0).unwrap();
        r.registrar(&modelo_4b(), 1).unwrap();
        for i in 0..50 {
            r.crear_conversacion(&format!("Conversación {i}"), i)
                .unwrap();
        }
    } // closing it moves everything from the -wal to the main file
    // Overwrite the second page (4096 bytes after the header page) with garbage, as a disk failure would.
    let mut bytes = std::fs::read(&db.0).unwrap();
    assert!(bytes.len() > 8192, "la base debería tener varias páginas");
    bytes[4096..8192].fill(0xAB);
    std::fs::write(&db.0, &bytes).unwrap();

    let r = Registro::abrir(&db.0).unwrap();
    let apartada = r.base_apartada().expect("debería detectar el daño");
    assert_eq!(
        std::fs::read(apartada).unwrap(),
        bytes,
        "se aparta tal cual, sin tocarla"
    );
    assert!(r.listar().unwrap().is_empty(), "se empieza una nueva");
}

#[test]
fn una_base_de_datos_sana_no_se_aparta() {
    let db = BaseDeDatosDePrueba::nueva("sana");
    drop(Registro::abrir(&db.0).unwrap());
    let r = Registro::abrir(&db.0).unwrap();
    assert!(r.base_apartada().is_none());
    assert_eq!(db.buscar(".danada-"), None);
}

/// A database at version 2 with one model, and a list of migrations whose third one fails.
fn v2_con_una_migracion_que_falla(
    db: &BaseDeDatosDePrueba,
) -> (Connection, PathBuf, ErrorRegistro) {
    {
        let mut r = Registro::abrir(&db.0).unwrap();
        r.registrar(&modelo_4b(), 1).unwrap();
    }
    let mut c = Connection::open(&db.0).unwrap();
    let copia = db.0.with_extension("db.copia-antes-de-v3");
    let migraciones = [MIGRACIONES[0], MIGRACIONES[1], "ESTO NO ES SQL;"];
    let error = migrar(&mut c, &migraciones, Some(&copia)).unwrap_err();
    (c, copia, error)
}

#[test]
fn si_una_migracion_falla_se_conserva_la_copia_de_antes() {
    let db = BaseDeDatosDePrueba::nueva("migracion-falla");
    let (c, copia, error) = v2_con_una_migracion_que_falla(&db);
    match &error {
        ErrorRegistro::MigracionFallida { copia: dicha, .. } => assert_eq!(dicha, &copia),
        otro => panic!("error inesperado: {otro}"),
    }
    assert!(error.to_string().contains("copia"), "{error}");

    // The database is still at version 2 (the failed migration was rolled back)…
    let version: u32 = c
        .pragma_query_value(None, "user_version", |f| f.get(0))
        .unwrap();
    assert_eq!(version, 2);
    // …and the copy is a complete, valid database with the data from before.
    let guardada = Connection::open(&copia).unwrap();
    let modelos: i64 = guardada
        .query_row("SELECT count(*) FROM modelos", [], |f| f.get(0))
        .unwrap();
    assert_eq!(modelos, 1);
}

#[test]
fn una_copia_de_un_intento_anterior_no_se_sobrescribe() {
    let db = BaseDeDatosDePrueba::nueva("copia-anterior");
    let (mut c, copia, _) = v2_con_una_migracion_que_falla(&db);
    let antes = std::fs::read(&copia).unwrap();
    // Meanwhile the database changes, and the migration fails again.
    c.execute("DELETE FROM modelos", []).unwrap();
    let migraciones = [MIGRACIONES[0], MIGRACIONES[1], "ESTO NO ES SQL;"];
    assert!(migrar(&mut c, &migraciones, Some(&copia)).is_err());
    assert_eq!(
        std::fs::read(&copia).unwrap(),
        antes,
        "la copia más antigua se conserva"
    );
}

#[test]
fn una_base_nueva_no_hace_copia() {
    let db = BaseDeDatosDePrueba::nueva("nueva-sin-copia");
    drop(Registro::abrir(&db.0).unwrap());
    assert_eq!(db.buscar(".copia-antes-de-"), None);
}

#[test]
fn guarda_y_lee_conversaciones() {
    let mut r = Registro::en_memoria().unwrap();
    assert_eq!(r.version_esquema().unwrap(), 2);
    let c = r
        .crear_conversacion("  Plan de\nestudio   de Rust ", 100)
        .unwrap();
    assert_eq!(
        c.titulo, "Plan de estudio de Rust",
        "una sola línea, sin espacios de más"
    );
    assert_eq!((c.mensajes, c.creada, c.actualizada), (0, 100, 100));

    r.ajustar_conversacion(c.id, "qwen3-4b", "Responde en español.", Some(0.7))
        .unwrap();
    let pregunta = r
        .anadir_mensaje(
            c.id,
            &MensajeNuevo::del_usuario("¿Qué es el ownership?"),
            110,
        )
        .unwrap();
    let contestacion = r
        .anadir_mensaje(c.id, &respuesta("Cada valor tiene un dueño."), 120)
        .unwrap();
    assert_eq!((pregunta.orden, contestacion.orden), (0, 1));

    let leida = r.conversacion(c.id).unwrap().unwrap();
    assert_eq!(leida.mensajes, 2);
    assert_eq!(leida.actualizada, 120, "se actualiza al añadir mensajes");
    assert_eq!(leida.modelo.as_deref(), Some("qwen3-4b"));
    assert_eq!(leida.sistema, "Responde en español.");
    assert_eq!(leida.temperatura, Some(0.7));

    let mensajes = r.mensajes(c.id).unwrap();
    assert_eq!(mensajes.len(), 2);
    assert_eq!(mensajes[0].autor, Autor::Usuario);
    assert_eq!(mensajes[1], contestacion);

    r.renombrar_conversacion(c.id, "Rust desde cero").unwrap();
    assert_eq!(
        r.conversacion(c.id).unwrap().unwrap().titulo,
        "Rust desde cero"
    );
}

#[test]
fn ordena_por_la_mas_reciente_y_quita_el_ultimo_mensaje() {
    let mut r = Registro::en_memoria().unwrap();
    let vieja = r.crear_conversacion("Vieja", 10).unwrap();
    let nueva = r.crear_conversacion("Nueva", 20).unwrap();
    let ids: Vec<i64> = r.conversaciones().unwrap().iter().map(|c| c.id).collect();
    assert_eq!(ids, [nueva.id, vieja.id]);

    // Writing to the old one moves it to the top.
    r.anadir_mensaje(vieja.id, &MensajeNuevo::del_usuario("hola"), 30)
        .unwrap();
    assert_eq!(r.conversaciones().unwrap()[0].id, vieja.id);

    r.anadir_mensaje(vieja.id, &MensajeNuevo::del_usuario("otra"), 31)
        .unwrap();
    r.quitar_ultimo_mensaje(vieja.id).unwrap();
    let mensajes = r.mensajes(vieja.id).unwrap();
    assert_eq!(mensajes.len(), 1);
    assert_eq!(mensajes[0].contenido, "hola");
}

#[test]
fn errores_con_conversaciones_que_no_existen() {
    let mut r = Registro::en_memoria().unwrap();
    assert!(r.conversacion(99).unwrap().is_none());
    assert!(matches!(
        r.renombrar_conversacion(99, "x"),
        Err(ErrorRegistro::NoEncontrado(_))
    ));
    assert!(matches!(
        r.anadir_mensaje(99, &MensajeNuevo::del_usuario("x"), 1),
        Err(ErrorRegistro::NoEncontrado(_))
    ));
    assert!(matches!(
        r.eliminar_conversacion(99),
        Err(ErrorRegistro::NoEncontrado(_))
    ));
}

#[test]
fn los_titulos_largos_se_recortan() {
    let largo = "a".repeat(500);
    let t = titulo_valido(&largo);
    assert_eq!(t.chars().count(), TITULO_MAX + 1);
    assert!(t.ends_with('…'));
}

/// Does `texto` appear in any of the database files (the main one and the change log)?
fn esta_en_los_archivos(db: &BaseDeDatosDePrueba, texto: &str) -> bool {
    ["", "-wal"].iter().any(|sufijo| {
        let mut ruta = db.0.clone().into_os_string();
        ruta.push(sufijo);
        std::fs::read(ruta)
            .is_ok_and(|bytes| bytes.windows(texto.len()).any(|v| v == texto.as_bytes()))
    })
}

#[test]
fn eliminar_una_conversacion_la_borra_de_verdad_del_disco() {
    const SECRETO: &str = "ZANAHORIA-SECRETA-777";
    let db = BaseDeDatosDePrueba::nueva("borrado-real");
    let mut r = Registro::abrir(&db.0).unwrap();
    // Another conversation that stays, so the database is not left empty.
    let otra = r.crear_conversacion("Se queda", 1).unwrap();
    r.anadir_mensaje(otra.id, &MensajeNuevo::del_usuario("hola"), 2)
        .unwrap();

    let c = r
        .crear_conversacion(&format!("Sobre {SECRETO}"), 3)
        .unwrap();
    r.anadir_mensaje(
        c.id,
        &MensajeNuevo::del_usuario(format!("Mi clave es {SECRETO}")),
        4,
    )
    .unwrap();
    r.anadir_mensaje(c.id, &respuesta(&format!("Entendido: {SECRETO}")), 5)
        .unwrap();
    assert!(
        esta_en_los_archivos(&db, SECRETO),
        "la prueba debe ver el texto antes de borrarlo"
    );

    r.eliminar_conversacion(c.id).unwrap();

    assert!(
        !esta_en_los_archivos(&db, SECRETO),
        "el texto borrado sigue en el disco"
    );
    assert!(r.conversacion(c.id).unwrap().is_none());
    assert_eq!(
        r.mensajes(c.id).unwrap().len(),
        0,
        "los mensajes se borran con la conversación"
    );
    assert_eq!(
        r.mensajes(otra.id).unwrap().len(),
        1,
        "las demás no se tocan"
    );
}
