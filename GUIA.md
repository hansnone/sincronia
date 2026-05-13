# Sincronia — Guía completa

Sincronización multipar de archivos en Windows 11: varios orígenes y destinos, rutas largas vía letras virtuales (`DefineDosDeviceW`), motor de copia optimizado para SMB (incluido destinos en **macOS con APFS** compartido por SMB).

---

## Tabla de contenidos

1. [Requisitos](#requisitos)
2. [Ejecución con Cargo (desarrollo)](#ejecución-con-cargo)
3. [Compilación y obtención del EXE](#compilación-del-exe)
4. [Configuración](#configuración)
5. [Tarea programada de Windows](#tarea-programada)
6. [Despliegue en un equipo de producción](#despliegue)
7. [Arquitectura del sistema](#arquitectura)
8. [Referencia de módulos](#referencia-de-módulos)
9. [Solución de problemas](#solución-de-problemas)

---

## Requisitos

- **Sistema operativo**: Windows 10/11 (x64)
- **Rust toolchain**: `stable-x86_64-pc-windows-msvc` (1.85+)
- **Rutas de destino**: deben ser accesibles desde Windows como rutas normales o UNC (por ejemplo `\\servidor\carpeta` o un volumen ya mapeado). Sincronia **no** monta el NAS por WNet: tú defines la ruta real en el TOML y la asocias a una **letra virtual** para la sesión de copia.
- **Letras de unidad virtuales**: libres en el momento de la ejecución (no usadas por otras aplicaciones ni por otro par en el mismo TOML).
- **RAM**: ~128 MiB base + (workers × buffer). Con 8 workers × 16 MiB ≈ 256 MiB total
- **No requiere privilegios de administrador** (salvo crear la tarea programada)

---

## Ejecución con Cargo

### Primer paso: generar la configuración

```powershell
cd C:\Users\thebe\Desktop\Proyectos\sincronia
cargo run -- --generate-config sincronia.toml
```

Esto crea `sincronia.toml` con todos los parámetros documentados. **Edítalo** antes de ejecutar: al menos cada bloque `[[sync_pairs]]` (`source_path`, `target_path`, letras virtuales) y las rutas de logging.

### Ejecutar en modo normal (con icono en bandeja)

```powershell
cargo run -- --config sincronia.toml
```

Aparecerá un icono en la bandeja del sistema (círculo verde/amarillo/rojo según el estado). Clic derecho: pausar, reanudar, abrir logs, lista estática de pares configurados, etc.

### Ejecutar sin bandeja (modo depuración con logs en consola)

```powershell
cargo run -- --config sincronia.toml --no-tray
```

Útil para ver la salida de `tracing` en tiempo real durante el desarrollo.

### Opciones disponibles

| Opción | Descripción |
|--------|-------------|
| `--config <ruta>` | Ruta al archivo TOML (por defecto: `sincronia.toml` junto al .exe) |
| `--generate-config [ruta]` | Genera un archivo de configuración de ejemplo |
| `--create-scheduled-task` | Crea la tarea programada de Windows |
| `--no-tray` | Ejecutar sin icono en bandeja (útil para depuración) |
| `--version`, `-v` | Muestra la versión |
| `--help`, `-h` | Muestra la ayuda |

---

## Compilación del EXE

### Build de release (optimizado)

```powershell
cd C:\Users\thebe\Desktop\Proyectos\sincronia
cargo build --release
```

El ejecutable se genera en:

```
target\release\sincronia.exe
```

### Características del build release

El perfil de release en `Cargo.toml` está configurado para máximo rendimiento:

- `opt-level = 3` — optimización máxima
- `lto = "thin"` — Link-Time Optimization
- `codegen-units = 1` — mejor optimización a costa de tiempo de compilación
- `strip = "symbols"` — elimina símbolos de debug (EXE más pequeño)

### Verificar que compila correctamente

```powershell
cargo check
cargo build --release
```

---

## Configuración

El archivo `sincronia.toml` controla todo el comportamiento. Secciones principales:

### [general]

- `application_name`: nombre en bandeja y consola
- `run_mode`: `"backup_append_only"` (respaldar sin borrar origen) o `"move_after_verified_copy"` (mover tras verificar hash)

### [scan]

Intervalos **después de recorrer todos los pares** en una vuelta:

- `scan_interval_seconds_when_no_changes`: si en ningún par hubo archivos estables listos para copiar (por defecto 15)
- `scan_interval_seconds_after_changes`: si hubo trabajo estable en algún par (por defecto 10)

### [[sync_pairs]] (uno o más bloques)

Cada par define una sincronización independiente:

| Campo | Descripción |
|-------|-------------|
| `source_path` | Carpeta **real** de origen en disco (Windows) |
| `source_virtual_drive_letter` | Letra DOS virtual para esa carpeta en esta sesión (ej. `"X:"`) |
| `target_path` | Carpeta **real** de destino (puede ser UNC SMB, carpeta local, etc.) |
| `target_virtual_drive_letter` | Otra letra virtual para el destino (ej. `"Y:"`), **distinta** de la de origen |
| `minimum_file_stable_seconds` | Segundos sin cambios de tamaño/mtime para considerar el archivo estable (por defecto 60) |

En tiempo de ejecución, el orquestador:

1. Desmonta de forma preventiva ambas letras (`DefineDosDeviceW` con `DDD_REMOVE_DEFINITION`)
2. Monta `source_virtual_drive_letter` → `source_path` y `target_virtual_drive_letter` → `target_path`
3. Escanea y copia usando **solo** las rutas bajo esas letras (evita límites de longitud y simplifica rutas)
4. Desmonta ambas letras antes de pasar al siguiente par

La validación exige que existan los orígenes, que el destino exista o que exista su directorio padre, que las letras tengan formato `L:` y que **no se repitan letras** entre todos los pares.

**Rutas Windows en TOML:** en cadenas entre comillas dobles (`"..."`), la secuencia `\U` inicia un carácter Unicode de 8 dígitos hexadecimales. Por eso `"C:\Users\..."` falla al llegar a `\Users`. Soluciones: duplicar cada barra (`"C:\\Users\\thebe\\..."`) o usar una **cadena literal** de TOML entre comillas simples (`'C:\Users\thebe\...'`), donde `\` no es especial.

### [copy_engine]

- `worker_count`: hilos de copia concurrentes (por defecto 8)
- `copy_buffer_size_mib_per_worker`: buffer por worker en MiB (por defecto 16)
- `temporary_destination_extension`: sufijo del temporal (por defecto `.partial`)

### [verification]

- `hash_algorithm`: `"blake3"` o `"sha256"`
- `verification_mode`: `"full_hash"` o `"none"`

### [metadata]

- **`preserve_file_attributes`**: por defecto **`false`**. En NAS SMB servidos por **macOS/APFS**, copiar atributos NTFS (p. ej. solo lectura) al destino suele provocar bloqueos en el servidor e impedir renombrar el `.partial` al nombre final. Solo actívalo si conoces el comportamiento del destino.
- Otros campos: timestamps (creación, modificación, último acceso), etc.

### [exclusions]

- `excluded_directory_names`: carpetas a ignorar por nombre
- `excluded_file_patterns`: globs sobre el nombre del archivo; por defecto incluye entre otros `*.partial`, `.DS_Store`, `._*` (metadatos de macOS en SMB)

### [logging]

- Tres archivos: `.log` (humano), `.csv` (métricas), `.jsonl` (eventos)
- Se crean automáticamente los directorios padre si no existen

### [scheduled_task]

- `delay_after_logon_seconds`: espera tras el inicio de sesión antes de arrancar (por defecto 30)
- `run_only_when_user_is_logged_on`: suele ser `true` si quieres bandeja e interacción en la sesión del usuario

> **Referencia completa**: consulta `sincronia.example.toml` en el repositorio (todos los parámetros comentados).

---

## Tarea programada

### Crear la tarea (una sola vez)

Asegúrate de que la sección `[scheduled_task]` del TOML está configurada y ejecuta:

```powershell
cargo run -- --config sincronia.toml --create-scheduled-task
```

o con el EXE compilado:

```powershell
sincronia.exe --config C:\Sincronia\sincronia.toml --create-scheduled-task
```

Comportamiento típico (según el TOML de ejemplo):

1. Se dispara al iniciar sesión el usuario actual
2. Espera unos segundos para que el sistema y la red estén listos
3. Ejecuta `sincronia.exe --config <ruta>`
4. Puede impedir instancias paralelas

### Verificar la tarea

```powershell
Get-ScheduledTask -TaskName "Sincronia NAS Backup"
taskschd.msc
```

(El nombre exacto es el de `scheduled_task_name` en el TOML.)

### Eliminar la tarea

```powershell
Unregister-ScheduledTask -TaskName "Sincronia NAS Backup" -Confirm:$false
```

### Parámetros útiles

| Parámetro | Valor recomendado | Motivo |
|-----------|-------------------|--------|
| `run_only_when_user_is_logged_on` | `true` | Bandeja y sesión interactiva del usuario |
| `run_with_highest_privileges` | `false` | No suele ser necesario |
| `delay_after_logon_seconds` | `30` | Da tiempo a que la red y las unidades estén disponibles |

---

## Despliegue

### Pasos sugeridos en producción

1. Compilar en desarrollo: `cargo build --release`
2. Copiar al equipo destino:
   - `sincronia.exe` → por ejemplo `C:\Sincronia\sincronia.exe`
   - `sincronia.toml` → `C:\Sincronia\sincronia.toml`
3. Editar `sincronia.toml`:
   - Uno o más `[[sync_pairs]]` con rutas reales y letras virtuales **únicas** y libres
   - `[scan]` según la frecuencia deseada entre vueltas completas
   - Rutas de `[logging]` adaptadas al usuario
4. Crear la tarea programada (opcional):  
   `C:\Sincronia\sincronia.exe --config C:\Sincronia\sincronia.toml --create-scheduled-task`
5. Probar manualmente:  
   `C:\Sincronia\sincronia.exe --config C:\Sincronia\sincronia.toml`
6. Si usas tarea al inicio de sesión: cerrar sesión y volver a entrar para comprobar el arranque automático

### Estructura de archivos en producción

```
C:\Sincronia\
├── sincronia.exe
├── sincronia.toml
└── (los logs donde indique el TOML)

C:\Users\<usuario>\Documents\Sincronia\Logs\
├── sincronia.log
├── sincronia-metrics.csv
└── sincronia-events.jsonl
```

---

## Arquitectura

### Diagrama de flujo (resumen)

```
main → Orchestrator (bucle global)
         │
         ├─► por cada [[sync_pairs]]:
         │     montar letras DOS → escanear origen virtual → estabilidad
         │     → planificar → WorkerPool → métricas → desmontar letras
         │
         ├─► Idle + espera [scan]
         │
         └─► Tray (canales crossbeam): estado, notificaciones, ciclo completado
```

### Ciclo del orquestador

En cada vuelta del bucle principal:

1. Comprobar parada y comandos de la bandeja (pausa / reanudar / salir)
2. Para **cada** par en `sync_pairs`:
   - Limpiar y montar letras virtuales con `DefineDosDeviceW`
   - Si el montaje falla: registrar, desmontar lo montado y **seguir con el siguiente par**
   - Escanear la raíz **virtual** de origen (p. ej. `X:\`)
   - Evaluar estabilidad (un `StabilityChecker` por índice de par)
   - Si hay archivos estables: planificar hacia la base **virtual** de destino (p. ej. `Y:\`), ejecutar el pool, registrar métricas, opcionalmente vaciar directorios vacíos en el origen virtual
   - Desmontar **siempre** ambas letras al terminar el par
3. Estado `Idle` y `interruptible_sleep` según `[scan]` y si hubo trabajo estable en la vuelta

### Pipeline por archivo (en cada worker)

1. Resolución de conflictos (hash, versionado, etc.)
2. Copia a temporal (`.partial`): buffer reutilizable, en Windows `FILE_FLAG_SEQUENTIAL_SCAN`, `FlushFileBuffers` / `sync_all`
3. Tras el sync: **cierre explícito** de lectura/escritura (`drop`) antes de medir tiempos (reduce ventanas de bloqueo con antivirus/SMB)
4. Verificación de hash si está configurada
5. Metadatos (respetando `preserve_file_attributes`; por defecto desactivado para compatibilidad con destinos macOS/APFS)
6. **Finalizar**: `rename` de `.partial` al nombre final con **hasta 5 reintentos** y **200 ms** entre intentos (latencia SMB en smbd/macOS)
7. Reintentos por archivo según `[retry_policy]`

### Bandeja del sistema

| Color | Significado (orientativo) |
|-------|---------------------------|
| Verde | Escaneando, copiando (con detalle del par), inactivo |
| Amarillo | Iniciando, cargando/validando configuración, pausado |
| Rojo | Error persistente, parando, parado |

Menú contextual:

- Estado (texto truncado si es muy largo)
- Líneas informativas **solo lectura** por cada par configurado en el TOML
- Pausar / Reanudar / Parar ordenadamente
- Abrir carpeta de logs, configuración, métricas
- Salir

---

## Referencia de módulos

| Módulo | Archivo | Función |
|--------|---------|---------|
| **main** | `main.rs` | Entrada: argumentos, carga de config, orquestador + bandeja |
| **config** | `config.rs` | Tipos TOML (`sync_pairs`, `scan`, motor, exclusiones, validación) |
| **orchestrator** | `orchestrator.rs` | Bucle global: montaje DOS por par, escaneo/copia en letras virtuales, desmontaje |
| **scanner** | `scanner.rs` | Recorrido recursivo con rutas largas (`\\?\`) cuando aplica |
| **stability** | `stability.rs` | Estabilidad por tamaño + mtime; caché de “ya respaldado” |
| **planner** | `planner.rs` | `CopyJob`: origen, destino, temporal |
| **scheduler** | `scheduler.rs` | Pool de workers y canales |
| **copy_engine** | `copy_engine.rs` | Copia en bloques, sync, drops explícitos, `finalize_copy` con reintentos |
| **verifier** | `verifier.rs` | Hash post-copia |
| **hasher** | `hasher.rs` | BLAKE3 / SHA-256 |
| **metadata** | `metadata.rs` | Timestamps y atributos vía API Windows |
| **conflict** | `conflict.rs` | Conflictos y nombres versionados |
| **exclusions** | `exclusions.rs` | Directorios y globs de archivo |
| **logging** | `logging.rs` | `.log`, `.csv`, `.jsonl` |
| **stats** | `stats.rs` | Métricas por ciclo y por archivo |
| **tray** | `tray.rs` | Icono y menú (tray-icon + winit) |
| **shutdown** | `shutdown.rs` | Parada ordenada y Ctrl+C |
| **scheduled_task** | `scheduled_task.rs` | Tarea programada vía PowerShell |
| **errors** | `errors.rs` | Errores, estados globales, modos |

---

## Solución de problemas

### No se montan las letras virtuales (`DefineDosDeviceW`)

- Comprueba que las letras del TOML **no** estén en uso (`subst`, otras apps, otro par con la misma letra).
- Revisa el log: mensajes de error del kernel al montar.
- Tras un fallo, el orquestador intenta desmontar y pasa al **siguiente par**; la vuelta completa puede reintentarse según `[scan]` y `[retry_policy].nas_retry_delay_seconds` (nombre histórico: pausa entre ciclos ante fallos).

### Rutas largas o permisos raros en Windows

- El montaje a `X:` / `Y:` acota rutas bajo esa raíz; el escáner normaliza con prefijo `\\?\` cuando corresponde.

### Destino macOS / APFS por SMB: “acceso denegado” al renombrar `.partial`

- Mantén **`preserve_file_attributes = false`** (por defecto) en `[metadata]`.
- El motor ya **reintenta** el `rename` varias veces con pausa corta; si sigue fallando, revisa espacio en disco, cuotas y permisos SMB en el Mac.

### Los archivos no se copian

- `minimum_file_stable_seconds`: el archivo debe permanecer sin cambios el tiempo indicado.
- Patrones en `excluded_file_patterns` (incluidos `.partial`, `._*`, `.DS_Store`, etc.).
- Logs en `human_log_file_path`.

### La tarea programada no arranca

```powershell
Get-ScheduledTask -TaskName "Sincronia NAS Backup" | Select-Object State, LastRunTime, LastTaskResult
Get-ScheduledTaskInfo -TaskName "Sincronia NAS Backup"
Start-ScheduledTask -TaskName "Sincronia NAS Backup"
```

(Ajusta el nombre al valor de `scheduled_task_name` en tu TOML.)

### Ver el log en tiempo real

```powershell
Get-Content "C:\Users\...\Sincronia\Logs\sincronia.log" -Wait -Tail 50
```

---

*Documento alineado con la versión multipar del proyecto (pares `[[sync_pairs]]`, montaje DOS y compatibilidad SMB/macOS).*
