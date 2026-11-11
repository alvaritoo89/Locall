// Home: your computer, the latest conversation and the installed models.

import { EstadoDelModelo, NombreDelModelo, ReglaDeMemoria } from "./comun";
import { type Conversacion, type EstadoLocall, type Modelo, haceCuanto, tamano } from "./locall";

interface Props {
  estado: EstadoLocall | null;
  error: string | null;
  modelos: Modelo[] | null;
  irABiblioteca: () => void;
  /** The most recent conversation, to continue it. */
  ultima: Conversacion | null;
  continuar: (id: number) => void;
}

export function Inicio({ estado, error, modelos, irABiblioteca, ultima, continuar }: Props) {
  return (
    <>
      <h1>Inicio</h1>
      {error && !estado && (
        <section className="tarjeta aviso" role="alert">
          <h2>No se puede conectar con Locall</h2>
          <p>{error}</p>
        </section>
      )}
      {!estado && !error && <p className="tenue">Abriendo Locall…</p>}
      {estado && (
        <section className="tarjeta">
          <div className="titulo-tarjeta">
            <h2>Tu equipo</h2>
            <span className="tenue pequeño">Detectado ahora</span>
          </div>
          <div className="datos">
            <Dato nombre="Procesador" valor={estado.cpu ?? "Desconocido"} />
            <Dato nombre="Hilos" valor={String(estado.threads)} />
            <Dato nombre="Memoria" valor={tamano(estado.memoryTotal)} />
          </div>
          <ReglaDeMemoria estado={estado} />
          {estado.warnings.map((aviso) => (
            <p key={aviso} className="pequeño tono-aviso">Aviso: {aviso}</p>
          ))}
        </section>
      )}

      {ultima && (
        <section className="tarjeta continuar">
          <div className="titulo-tarjeta">
            <h2>Continuar</h2>
            <span className="tenue pequeño">{haceCuanto(ultima.updated_at)}</span>
          </div>
          <p className="titulo-continuar">{ultima.title}</p>
          <p className="pequeño tenue">
            {ultima.message_count} {ultima.message_count === 1 ? "mensaje" : "mensajes"}
            {ultima.model && <> · <span className="mono">{ultima.model}</span></>}
          </p>
          <button type="button" className="boton" onClick={() => continuar(ultima.id)}>
            Continuar la conversación →
          </button>
        </section>
      )}

      <section className="tarjeta ancha sin-relleno">
        <div className="cabecera-tabla">
          <h2>Modelos instalados</h2>
          <button type="button" className="enlace" onClick={irABiblioteca}>
            Ir a la biblioteca →
          </button>
        </div>
        {modelos === null && <p className="tenue relleno">Cargando la lista…</p>}
        {modelos?.length === 0 && (
          <p className="tenue relleno">
            Todavía no tienes modelos. Instala uno desde la{" "}
            <button type="button" className="enlace" onClick={irABiblioteca}>biblioteca</button>.
          </p>
        )}
        {modelos && modelos.length > 0 && (
          <table className="tabla">
            <thead>
              <tr>
                <th scope="col">Modelo</th>
                <th scope="col">Tamaño</th>
                <th scope="col">Licencia</th>
                <th scope="col">Estado</th>
              </tr>
            </thead>
            <tbody>
              {modelos.map((m) => (
                <tr key={m.id}>
                  <td><NombreDelModelo modelo={m} /></td>
                  <td className="mono">{tamano(m.size_bytes)}</td>
                  <td>{m.license ?? "—"}</td>
                  <td><EstadoDelModelo modelo={m} estado={estado} /></td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>
    </>
  );
}

function Dato({ nombre, valor }: { nombre: string; valor: string }) {
  return (
    <div className="dato">
      <span className="tenue pequeño">{nombre}</span>
      <span className="valor">{valor}</span>
    </div>
  );
}
