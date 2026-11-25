(function createNeoPath(host) {
    "use strict";

    const NativeTypeError = TypeError;
    if (!host || typeof host.isWindows !== "boolean" || typeof host.readCwd !== "function") {
        throw new NativeTypeError(
            "neo:path host requires an isWindows boolean and a readCwd function",
        );
    }

    const hostWindows = host.isWindows;
    const rawReadCwd = host.readCwd;
    function readCwd() {
        const directory = rawReadCwd();
        return directory === undefined ? null : directory;
    }

    // The host builds this API in the fresh realm, before running user code.
    const NativeError = Error;
    const defineProperty = Object.defineProperty;
    const freeze = Object.freeze;
    const isArray = Array.isArray;
    const isSafeInteger = Number.isSafeInteger;
    const charCodeAt = Function.prototype.call.bind(String.prototype.charCodeAt);
    const slice = Function.prototype.call.bind(String.prototype.slice);
    const split = Function.prototype.call.bind(String.prototype.split);
    const lower = Function.prototype.call.bind(String.prototype.toLowerCase);
    const indexOf = Function.prototype.call.bind(String.prototype.indexOf);
    const lastIndexOf = Function.prototype.call.bind(String.prototype.lastIndexOf);
    const joinArray = Function.prototype.call.bind(Array.prototype.join);
    const sliceArray = Function.prototype.call.bind(Array.prototype.slice);

    const SLASH = 47;
    const BACKSLASH = 92;
    const DOT = 46;
    const COLON = 58;

    function isPosixSeparator(code) {
      return code === SLASH;
    }

    function isWin32Separator(code) {
      return code === SLASH || code === BACKSLASH;
    }

    /** True for an ASCII letter, the only drive-letter form Windows accepts. */
    function isDeviceRoot(code) {
      return (code >= 65 && code <= 90) || (code >= 97 && code <= 122);
    }

    // True when the whole path is a recognized UNC root: exactly two leading
    // separators, a non-empty server, at least one separator, a non-empty share,
    // and nothing after the share except separators. This is the same root that
    // `parse` reports as `{root, dir, base: '', ext: '', name: ''}`. A run of
    // three or more leading separators is not a UNC root.
    function isWin32UncRootOnly(path) {
      const length = path.length;
      if (length < 4) return false;
      if (
        !isWin32Separator(charCodeAt(path, 0)) ||
        !isWin32Separator(charCodeAt(path, 1))
      ) {
        return false;
      }

      let index = 2;
      const serverStart = index;
      while (index < length && !isWin32Separator(charCodeAt(path, index))) ++index;
      if (index === serverStart || index >= length) return false;

      const separatorStart = index;
      while (index < length && isWin32Separator(charCodeAt(path, index))) ++index;
      if (index === separatorStart || index >= length) return false;

      const shareStart = index;
      while (index < length && !isWin32Separator(charCodeAt(path, index))) ++index;
      if (index === shareStart) return false;

      while (index < length) {
        if (!isWin32Separator(charCodeAt(path, index))) return false;
        ++index;
      }
      return true;
    }

    function assertString(value, name) {
      if (typeof value !== 'string')
        throw new NativeTypeError(name + ' must be a string');
      if (indexOf(value, '\u0000') !== -1)
        throw new NativeTypeError(name + ' must not contain a NUL character');
      return value;
    }

    function assertPath(value) {
      return assertString(value, 'path');
    }

    function assertSuffix(value) {
      return assertString(value, 'suffix');
    }

    // Resolve `.` and `..` segments. `allowAboveRoot` decides whether a `..` with
    // no preceding segment is kept. A trailing separator is not handled here.
    function normalizeString(path, allowAboveRoot, separator, isSeparator) {
      let result = '';
      let lastSegmentLength = 0;
      let lastSlash = -1;
      let dots = 0;
      let code;

      for (let i = 0; i <= path.length; ++i) {
        if (i < path.length) code = charCodeAt(path, i);
        else if (isSeparator(code)) break;
        else code = SLASH;

        if (isSeparator(code)) {
          if (lastSlash === i - 1 || dots === 1) {
            // Empty segment or `.`: nothing to add.
          } else if (lastSlash !== i - 1 && dots === 2) {
            if (
              result.length < 2 ||
              lastSegmentLength !== 2 ||
              charCodeAt(result, result.length - 1) !== DOT ||
              charCodeAt(result, result.length - 2) !== DOT
            ) {
              if (result.length > 2) {
                const lastSlashIndex = lastIndexOf(result, separator);
                if (lastSlashIndex === -1) {
                  result = '';
                  lastSegmentLength = 0;
                } else {
                  result = slice(result, 0, lastSlashIndex);
                  lastSegmentLength =
                    result.length - 1 - lastIndexOf(result, separator);
                }
                lastSlash = i;
                dots = 0;
                continue;
              }
              if (result.length === 2 || result.length === 1) {
                result = '';
                lastSegmentLength = 0;
                lastSlash = i;
                dots = 0;
                continue;
              }
            }
            if (allowAboveRoot) {
              result = result.length > 0 ? result + separator + '..' : '..';
              lastSegmentLength = 2;
            }
          } else {
            const segment = slice(path, lastSlash + 1, i);
            result = result.length > 0 ? result + separator + segment : segment;
            lastSegmentLength = i - lastSlash - 1;
          }
          lastSlash = i;
          dots = 0;
        } else if (code === DOT && dots !== -1) {
          ++dots;
        } else {
          dots = -1;
        }
      }

      return result;
    }

    // Remove trailing separators without shortening the root. `minimum` is the
    // number of leading code units that belong to the root.
    function stripTrailingSeparators(segment, isSeparator, minimum) {
      let end = segment.length;
      for (let i = segment.length - 1; i >= minimum; --i) {
        if (!isSeparator(charCodeAt(segment, i))) break;
        end = i;
      }
      return slice(segment, 0, end);
    }

    function stripSuffix(name, suffix) {
      if (suffix.length === 0 || suffix.length >= name.length) return name;
      const offset = name.length - suffix.length;
      for (let i = suffix.length - 1; i >= 0; --i) {
        if (charCodeAt(name, offset + i) !== charCodeAt(suffix, i)) return name;
      }
      return slice(name, 0, -suffix.length);
    }

    function basenamePath(path, suffix, isSeparator, hasDrivePrefix) {
      let end = path.length;
      while (end > 0 && isSeparator(charCodeAt(path, end - 1))) end--;

      let start = 0;
      if (
        hasDrivePrefix &&
        end >= 2 &&
        isDeviceRoot(charCodeAt(path, 0)) &&
        charCodeAt(path, 1) === COLON
      ) {
        start = 2;
      }

      if (end <= start) return '';

      let nameStart = end - 1;
      while (nameStart > start && !isSeparator(charCodeAt(path, nameStart - 1)))
        nameStart--;

      return stripSuffix(slice(path, nameStart, end), suffix);
    }

    function extnamePath(path, isSeparator, start) {
      let startDot = -1;
      let startPart = start;
      let end = -1;
      let matchedSlash = true;
      let preDotState = 0;

      for (let i = path.length - 1; i >= start; --i) {
        const code = charCodeAt(path, i);
        if (isSeparator(code)) {
          if (!matchedSlash) {
            startPart = i + 1;
            break;
          }
          continue;
        }
        if (end === -1) {
          matchedSlash = false;
          end = i + 1;
        }
        if (code === DOT) {
          if (startDot === -1) startDot = i;
          else if (preDotState !== 1) preDotState = 1;
        } else if (startDot !== -1) {
          preDotState = -1;
        }
      }

      if (
        startDot === -1 ||
        end === -1 ||
        preDotState === 0 ||
        (preDotState === 1 && startDot === end - 1 && startDot === startPart + 1)
      ) {
        return '';
      }
      return slice(path, startDot, end);
    }

    function formatPath(separator, pathObject) {
      if (
        pathObject === null ||
        typeof pathObject !== 'object' ||
        isArray(pathObject)
      ) {
        throw new NativeTypeError('pathObject must be an object');
      }

      const root = readField(pathObject, 'root');
      const dirValue = readField(pathObject, 'dir');
      const baseValue = readField(pathObject, 'base');
      const name = readField(pathObject, 'name');
      const ext = readField(pathObject, 'ext');

      const dir = dirValue.length > 0 ? dirValue : root;
      const base = baseValue.length > 0 ? baseValue : name + ext;

      if (dir.length === 0) return base;
      if (dir === root) return dir + base;
      return dir + separator + base;
    }

    function readField(pathObject, name) {
      const value = pathObject[name];
      if (value === undefined) return '';
      return assertString(value, name);
    }

    function normalizePosix(path) {
      return normalizePosixText(assertPath(path));
    }

    function normalizePosixText(path) {
      const absolute = path.length > 0 && isPosixSeparator(charCodeAt(path, 0));
      const trailing =
        path.length > 0 && isPosixSeparator(charCodeAt(path, path.length - 1));

      let tail = normalizeString(path, !absolute, '/', isPosixSeparator);
      if (tail.length === 0 && !absolute) tail = '.';
      if (tail.length > 0 && trailing) tail += '/';

      return absolute ? '/' + tail : tail;
    }

    function joinPosix(...paths) {
      const parts = [];
      for (let i = 0; i < paths.length; i++) {
        const part = assertPath(paths[i]);
        if (part.length > 0) parts[parts.length] = part;
      }
      if (parts.length === 0) return '.';
      return normalizePosix(joinArray(parts, '/'));
    }

    function basenamePosix(path, suffix = '') {
      assertPath(path);
      assertSuffix(suffix);
      return basenamePath(path, suffix, isPosixSeparator, false);
    }

    function dirnamePosix(path) {
      assertPath(path);

      let end = -1;
      let matchedNonSeparator = false;
      for (let i = path.length - 1; i >= 1; --i) {
        if (isPosixSeparator(charCodeAt(path, i))) {
          if (matchedNonSeparator) {
            end = i;
            break;
          }
        } else {
          matchedNonSeparator = true;
        }
      }

      const absolute = path.length > 0 && isPosixSeparator(charCodeAt(path, 0));
      if (end === -1) return absolute ? '/' : '.';

      return stripTrailingSeparators(
        slice(path, 0, end),
        isPosixSeparator,
        absolute ? 1 : 0,
      );
    }

    function extnamePosix(path) {
      assertPath(path);
      return extnamePath(path, isPosixSeparator, 0);
    }

    function isAbsolutePosix(path) {
      assertPath(path);
      return path.length > 0 && isPosixSeparator(charCodeAt(path, 0));
    }

    function parsePosix(path) {
      assertPath(path);

      const result = { root: '', dir: '', base: '', ext: '', name: '' };
      if (path.length === 0) return result;

      const absolute = isPosixSeparator(charCodeAt(path, 0));
      let start;
      if (absolute) {
        result.root = '/';
        start = 1;
      } else {
        start = 0;
      }

      let startDot = -1;
      let startPart = 0;
      let end = -1;
      let matchedSlash = true;
      let preDotState = 0;

      for (let i = path.length - 1; i >= start; --i) {
        const code = charCodeAt(path, i);
        if (isPosixSeparator(code)) {
          if (!matchedSlash) {
            startPart = i + 1;
            break;
          }
          continue;
        }
        if (end === -1) {
          matchedSlash = false;
          end = i + 1;
        }
        if (code === DOT) {
          if (startDot === -1) startDot = i;
          else if (preDotState !== 1) preDotState = 1;
        } else if (startDot !== -1) {
          preDotState = -1;
        }
      }

      if (
        startDot === -1 ||
        end === -1 ||
        preDotState === 0 ||
        (preDotState === 1 && startDot === end - 1 && startDot === startPart + 1)
      ) {
        if (end !== -1) {
          const componentStart = startPart === 0 && absolute ? 1 : startPart;
          result.base = result.name = slice(path, componentStart, end);
        }
      } else {
        const componentStart = startPart === 0 && absolute ? 1 : startPart;
        result.name = slice(path, componentStart, startDot);
        result.base = slice(path, componentStart, end);
        result.ext = slice(path, startDot, end);
      }

      if (startPart > 0) {
        result.dir = stripTrailingSeparators(
          slice(path, 0, startPart - 1),
          isPosixSeparator,
          absolute ? 1 : 0,
        );
      } else if (absolute) {
        result.dir = '/';
      }

      return result;
    }

    function formatPosix(pathObject) {
      return formatPath('/', pathObject);
    }

    function normalizeWin32(path) {
      return normalizeWin32Text(assertPath(path));
    }

    function normalizeWin32Text(path) {
      const length = path.length;
      let rootEnd = 0;
      let device;
      let absolute = false;
      const code = charCodeAt(path, 0);

      if (length > 1) {
        if (isWin32Separator(code)) {
          // A leading separator is rooted. Two separators are a UNC root only when
          // a non-empty server and share follow; otherwise the run collapses.
          absolute = true;

          if (isWin32Separator(charCodeAt(path, 1))) {
            let j = 2;
            let last = j;
            while (j < length && !isWin32Separator(charCodeAt(path, j))) ++j;
            if (j < length && j !== last) {
              const firstPart = slice(path, last, j);
              last = j;
              while (j < length && isWin32Separator(charCodeAt(path, j))) ++j;
              if (j < length && j !== last) {
                last = j;
                while (j < length && !isWin32Separator(charCodeAt(path, j))) ++j;
                if (j === length) {
                  return '\\\\' + firstPart + '\\' + slice(path, last) + '\\';
                }
                if (j !== last) {
                  device = '\\\\' + firstPart + '\\' + slice(path, last, j);
                  rootEnd = j;
                }
              }
            }
          } else {
            rootEnd = 1;
          }
        } else if (
          isDeviceRoot(code) &&
          charCodeAt(path, 1) === COLON
        ) {
          device = slice(path, 0, 2);
          rootEnd = 2;
          if (length > 2) {
            if (isWin32Separator(charCodeAt(path, 2))) {
              absolute = true;
              rootEnd = 3;
            }
          }
        }
      } else if (isWin32Separator(code)) {
        return '\\';
      }

      let tail;
      if (rootEnd < length) {
        tail = normalizeString(
          slice(path, rootEnd),
          !absolute,
          '\\',
          isWin32Separator,
        );
      } else {
        tail = '';
      }

      if (tail.length === 0 && !absolute) tail = '.';
      if (tail.length > 0 && isWin32Separator(charCodeAt(path, length - 1)))
        tail += '\\';

      if (device === undefined) {
        if (absolute) return tail.length > 0 ? '\\' + tail : '\\';
        return tail;
      }
      if (absolute) return tail.length > 0 ? device + '\\' + tail : device + '\\';
      return device + tail;
    }

    function joinWin32(...paths) {
      const parts = [];
      for (let i = 0; i < paths.length; i++) {
        const part = assertPath(paths[i]);
        if (part.length > 0) parts[parts.length] = part;
      }
      return joinWin32Parts(parts);
    }

    function joinWin32Parts(parts) {
      if (parts.length === 0) return '.';

      let needsReplace = true;
      let slashCount = 0;
      const firstPart = parts[0];
      if (isWin32Separator(charCodeAt(firstPart, 0))) {
        ++slashCount;
        if (firstPart.length > 1) {
          if (isWin32Separator(charCodeAt(firstPart, 1))) {
            ++slashCount;
            if (firstPart.length > 2) {
              if (isWin32Separator(charCodeAt(firstPart, 2))) ++slashCount;
              else needsReplace = false;
            }
          }
        }
      }

      let joined = joinArray(parts, '\\');
      if (needsReplace) {
        for (; slashCount < joined.length; ++slashCount) {
          if (!isWin32Separator(charCodeAt(joined, slashCount))) break;
        }
        if (slashCount >= 2) joined = '\\' + slice(joined, slashCount);
      }

      return normalizeWin32(joined);
    }

    function basenameWin32(path, suffix = '') {
      assertPath(path);
      assertSuffix(suffix);
      // A complete UNC root is a directory, not a file, even when the share name
      // contains a dot. `parse` reports an empty `base` for the same path.
      if (isWin32UncRootOnly(path)) return '';
      return basenamePath(path, suffix, isWin32Separator, true);
    }

    function dirnameWin32(path) {
      assertPath(path);

      const length = path.length;
      let rootEnd = -1;
      let end = -1;
      let matchedSlash = true;
      let offset = 0;
      const code = charCodeAt(path, 0);

      if (length > 1) {
        if (isWin32Separator(code)) {
          rootEnd = offset = 1;

          if (isWin32Separator(charCodeAt(path, 1))) {
            let j = 2;
            let last = j;
            while (j < length && !isWin32Separator(charCodeAt(path, j))) ++j;
            if (j < length && j !== last) {
              last = j;
              while (j < length && isWin32Separator(charCodeAt(path, j))) ++j;
              if (j < length && j !== last) {
                last = j;
                while (j < length && !isWin32Separator(charCodeAt(path, j))) ++j;
                if (j === length) return path;
                if (j !== last) rootEnd = offset = j + 1;
              }
            }
          }
        } else if (isDeviceRoot(code) && charCodeAt(path, 1) === COLON) {
          rootEnd = offset = 2;
          if (length > 2 && isWin32Separator(charCodeAt(path, 2)))
            rootEnd = offset = 3;
        }
      } else if (isWin32Separator(code)) {
        return path;
      }

      for (let i = length - 1; i >= offset; --i) {
        if (isWin32Separator(charCodeAt(path, i))) {
          if (!matchedSlash) {
            end = i;
            break;
          }
        } else {
          matchedSlash = false;
        }
      }

      if (end === -1) {
        if (rootEnd === -1) return '.';
        return slice(path, 0, rootEnd);
      }

      return stripTrailingSeparators(
        slice(path, 0, end),
        isWin32Separator,
        rootEnd === -1 ? 0 : rootEnd,
      );
    }

    function extnameWin32(path) {
      assertPath(path);
      // The share name of a complete UNC root is not a file component, so a dot
      // in it is not an extension. This matches `parse`, which reports an empty
      // `base` for such a path.
      if (isWin32UncRootOnly(path)) return '';

      let start = 0;
      if (
        path.length >= 2 &&
        charCodeAt(path, 1) === COLON &&
        isDeviceRoot(charCodeAt(path, 0))
      ) {
        start = 2;
      }
      return extnamePath(path, isWin32Separator, start);
    }

    function isAbsoluteWin32(path) {
      assertPath(path);

      const length = path.length;
      if (length === 0) return false;

      const code = charCodeAt(path, 0);
      if (isWin32Separator(code)) return true;
      if (
        isDeviceRoot(code) &&
        length > 2 &&
        charCodeAt(path, 1) === COLON &&
        isWin32Separator(charCodeAt(path, 2))
      ) {
        return true;
      }
      return false;
    }

    function parseWin32(path) {
      assertPath(path);

      const result = { root: '', dir: '', base: '', ext: '', name: '' };
      const length = path.length;
      if (length === 0) return result;

      let rootEnd = 0;
      let code = charCodeAt(path, 0);

      if (length > 1) {
        if (isWin32Separator(code)) {
          rootEnd = 1;

          if (isWin32Separator(charCodeAt(path, 1))) {
            let j = 2;
            let last = j;
            while (j < length && !isWin32Separator(charCodeAt(path, j))) ++j;
            if (j < length && j !== last) {
              last = j;
              while (j < length && isWin32Separator(charCodeAt(path, j))) ++j;
              if (j < length && j !== last) {
                last = j;
                while (j < length && !isWin32Separator(charCodeAt(path, j))) ++j;
                if (j === length) {
                  rootEnd = j;
                } else if (j !== last) {
                  rootEnd = j + 1;
                }
              }
            }
          }
        } else if (isDeviceRoot(code) && charCodeAt(path, 1) === COLON) {
          rootEnd = 2;
          if (length > 2) {
            if (isWin32Separator(charCodeAt(path, 2))) {
              if (length === 3) {
                result.root = result.dir = path;
                return result;
              }
              rootEnd = 3;
            }
          } else {
            result.root = result.dir = path;
            return result;
          }
        }
      } else if (isWin32Separator(code)) {
        result.root = result.dir = path;
        return result;
      }

      if (rootEnd > 0) result.root = slice(path, 0, rootEnd);

      let startDot = -1;
      let startPart = rootEnd;
      let end = -1;
      let matchedSlash = true;
      let preDotState = 0;

      for (let i = length - 1; i >= rootEnd; --i) {
        code = charCodeAt(path, i);
        if (isWin32Separator(code)) {
          if (!matchedSlash) {
            startPart = i + 1;
            break;
          }
          continue;
        }
        if (end === -1) {
          matchedSlash = false;
          end = i + 1;
        }
        if (code === DOT) {
          if (startDot === -1) startDot = i;
          else if (preDotState !== 1) preDotState = 1;
        } else if (startDot !== -1) {
          preDotState = -1;
        }
      }

      if (
        startDot === -1 ||
        end === -1 ||
        preDotState === 0 ||
        (preDotState === 1 && startDot === end - 1 && startDot === startPart + 1)
      ) {
        if (end !== -1) result.base = result.name = slice(path, startPart, end);
      } else {
        result.name = slice(path, startPart, startDot);
        result.base = slice(path, startPart, end);
        result.ext = slice(path, startDot, end);
      }

      if (startPart > 0 && startPart !== rootEnd) {
        result.dir = stripTrailingSeparators(
          slice(path, 0, startPart - 1),
          isWin32Separator,
          rootEnd,
        );
      } else {
        result.dir = result.root;
      }

      return result;
    }

    function formatWin32(pathObject) {
      return formatPath('\\', pathObject);
    }

    /** Read one length from a possibly proxied array, without coercion. */
    function arrayLength(value, name) {
      if (!isArray(value)) throw new NativeTypeError(name + ' must be an array');
      const length = value.length;
      if (!isSafeInteger(length) || length < 0 || length > 0xffffffff)
        throw new NativeTypeError(name + ' length must be a valid array length');
      return length;
    }

    /** Find the shared spelling at component boundaries, without normalizing. */
    function commonPath(paths, separator) {
      const length = arrayLength(paths, 'paths');
      if (length === 0) return '';
      const first = split(assertPath(paths[0]), separator);
      let end = first.length;
      let append = '';
      for (let j = 1; j < length; ++j) {
        const parts = split(assertPath(paths[j]), separator);
        if (parts.length <= end) {
          end = parts.length;
          append = '';
        }
        for (let i = 0; i < end; ++i) {
          if (first[i] !== parts[i]) {
            end = i;
            append = i === 0 ? '' : separator;
            break;
          }
        }
      }
      return joinArray(sliceArray(first, 0, end), separator) + append;
    }

    function pathError(code, message) {
      const error = new NativeError(message);
      defineProperty(error, 'name', {
        __proto__: null,
        value: 'PathError',
        writable: true,
        enumerable: false,
        configurable: true,
      });
      defineProperty(error, 'code', {
        __proto__: null,
        value: code,
        writable: true,
        enumerable: true,
        configurable: true,
      });
      return error;
    }

    /** Obtain only the process cwd for the host dialect; never translate it. */
    function requireCwd(windows) {
      if (windows !== hostWindows) {
        throw pathError('PATH_CWD_DIALECT', 'provide an absolute base for the non-host path dialect');
      }
      const directory = readCwd();
      if (directory === null || directory === undefined) {
        throw pathError('PATH_CWD', 'the working directory is unavailable; provide an absolute base');
      }
      return directory;
    }

    /**
     * A lazy, per-call cwd snapshot for one dialect.
     *
     * `relative` resolves two endpoints; both must see the same directory
     * even if the host directory changes between reads. The first call that
     * actually needs a base reads cwd; later calls in the same operation
     * reuse it. A call whose inputs are all absolute never invokes it.
     */
    function cwdSnapshot(windows) {
      let read = false;
      let value;
      return function readCwdOnce() {
        if (!read) {
          value = requireCwd(windows);
          read = true;
        }
        return value;
      };
    }

    /** Resolve from the rightmost absolute segment, or one cwd snapshot. */
    function resolvePosix(...paths) {
      for (let i = 0; i < paths.length; ++i) assertPath(paths[i]);
      return resolvePosixInternal(paths, cwdSnapshot(false));
    }

    function resolvePosixInternal(paths, readCwdOnce) {
      const tails = [];
      for (let i = paths.length - 1; i >= -1; --i) {
        const path = i === -1 ? readCwdOnce() : paths[i];
        if (path === '') continue;
        tails[tails.length] = path;
        if (isPosixSeparator(charCodeAt(path, 0))) break;
      }
      return '/' + normalizedTail(tails, '/', isPosixSeparator);
    }

    function normalizedTail(tails, separator, isSeparator) {
      // Segments were collected right to left. Swap in place before normalizing.
      for (let i = 0, j = tails.length - 1; i < j; ++i, --j) {
        const tail = tails[i];
        tails[i] = tails[j];
        tails[j] = tail;
      }
      return normalizeString(joinArray(tails, separator), false, separator, isSeparator);
    }

    function skipWin32Separators(path, start) {
      while (isWin32Separator(charCodeAt(path, start))) ++start;
      return start;
    }

    function win32ComponentEnd(path, start) {
      while (start < path.length && !isWin32Separator(charCodeAt(path, start))) ++start;
      return start;
    }

    /** A namespace prefix such as `\\?\` or `\\.\`, including slash spellings. */
    function hasNamespacePrefix(path) {
      const marker = charCodeAt(path, 2);
      return isWin32Separator(charCodeAt(path, 0)) && isWin32Separator(charCodeAt(path, 1)) &&
        (marker === 63 || marker === DOT) && isWin32Separator(charCodeAt(path, 3));
    }

    /** Extract a device/share as a unit so `..` cannot climb above it. */
    function resolutionRootWin32(path) {
      const first = charCodeAt(path, 0);
      if (isDeviceRoot(first) && charCodeAt(path, 1) === COLON) {
        const device = slice(path, 0, 2);
        const absolute = isWin32Separator(charCodeAt(path, 2));
        return { device, drive: device, absolute, end: absolute ? 3 : 2 };
      }
      if (!isWin32Separator(first)) return { device: '', drive: '', absolute: false, end: 0 };
      if (isWin32Separator(charCodeAt(path, 1))) {
        const serverEnd = win32ComponentEnd(path, 2);
        const shareStart = skipWin32Separators(path, serverEnd);
        let end = win32ComponentEnd(path, shareStart);
        if (serverEnd > 2 && end > shareStart) {
          const server = slice(path, 2, serverEnd);
          const share = slice(path, shareStart, end);
          let device = '\\\\' + server + '\\' + share;
          let drive = '';
          if (server === '?' && lower(share) === 'unc') {
            const hostStart = skipWin32Separators(path, end);
            const hostEnd = win32ComponentEnd(path, hostStart);
            const volumeStart = skipWin32Separators(path, hostEnd);
            end = win32ComponentEnd(path, volumeStart);
            if (hostStart === hostEnd || volumeStart === end) {
              throw pathError('PATH_WINDOWS_ROOT', 'provide a server and share for a namespaced UNC path');
            }
            device += '\\' + slice(path, hostStart, hostEnd) + '\\' + slice(path, volumeStart, end);
          } else if (server === '?' && share.length === 2 &&
            isDeviceRoot(charCodeAt(share, 0)) && charCodeAt(share, 1) === COLON) {
            drive = share;
          }
          return { device, drive, absolute: true, end };
        }
      }
      if (hasNamespacePrefix(path)) {
        throw pathError('PATH_WINDOWS_ROOT', 'provide a complete Windows namespace root');
      }
      return { device: '', drive: '', absolute: true, end: 1 };
    }

    /** Resolve on one drive/share, without consulting any per-drive environment. */
    function resolveWin32(...paths) {
      for (let i = 0; i < paths.length; ++i) assertPath(paths[i]);
      return resolveWin32Internal(paths, cwdSnapshot(true));
    }

    function resolveWin32Internal(paths, readCwdOnce) {
      let device = '';
      let driveRelative = false;
      let absolute = false;
      const tails = [];
      for (let i = paths.length - 1; i >= -1; --i) {
        const path = i === -1 ? readCwdOnce() : paths[i];
        if (path === '') continue;
        const root = resolutionRootWin32(path);
        if (root.device !== '') {
          if (device === '') {
            device = root.device;
            driveRelative = !root.absolute;
          } else if (lower(device) !== lower(root.device)) {
            // Windows canonicalization can store a verbatim drive cwd. Match a
            // drive-relative input to this drive, but keep the verbatim spelling.
            if (driveRelative && lower(device) === lower(root.drive)) device = root.device;
            else continue;
          }
        }
        if (!absolute) {
          tails[tails.length] = slice(path, root.end);
          absolute = root.absolute;
        }
        if (absolute && device !== '') break;
      }
      if (!absolute || device === '') {
        throw pathError('PATH_WINDOWS_ROOT', 'provide an absolute Windows base on the requested drive or share');
      }
      return device + '\\' + normalizedTail(tails, '\\', isWin32Separator);
    }

    /** Compare whole components, retaining the destination's original spelling. */
    function relativeTail(from, to, separator, foldCase) {
      const source = from === '' ? [] : split(from, separator);
      const target = to === '' ? [] : split(to, separator);
      let i = 0;
      while (i < source.length && i < target.length &&
        (foldCase ? lower(source[i]) === lower(target[i]) : source[i] === target[i])) ++i;
      const result = [];
      for (let j = i; j < source.length; ++j) result[result.length] = '..';
      for (; i < target.length; ++i) result[result.length] = target[i];
      const path = joinArray(result, separator);
      // A destination component that starts with a drive letter and colon
      // must not turn this relative result into a drive-relative path.
      return foldCase && isDeviceRoot(charCodeAt(path, 0)) && charCodeAt(path, 1) === COLON
        ? '.\\' + path : path;
    }

    function relativePosix(from, to) {
      assertPath(from);
      assertPath(to);
      const readCwdOnce = cwdSnapshot(false);
      const source = resolvePosixInternal([from], readCwdOnce);
      const target = resolvePosixInternal([to], readCwdOnce);
      return relativeTail(slice(source, 1), slice(target, 1), '/', false);
    }

    function relativeWin32(from, to) {
      assertPath(from);
      assertPath(to);
      const readCwdOnce = cwdSnapshot(true);
      const source = resolveWin32Internal([from], readCwdOnce);
      const target = resolveWin32Internal([to], readCwdOnce);
      const sourceRoot = resolutionRootWin32(source);
      const targetRoot = resolutionRootWin32(target);
      if (lower(sourceRoot.device) !== lower(targetRoot.device)) return target;
      return relativeTail(
        slice(source, skipWin32Separators(source, sourceRoot.end)),
        slice(target, skipWin32Separators(target, targetRoot.end)),
        '\\',
        true,
      );
    }

    function toNamespacedPathWin32(path) {
      assertPath(path);
      if (path === '' || hasNamespacePrefix(path)) return path;
      const resolved = resolveWin32(path);
      if (hasNamespacePrefix(resolved)) return resolved;
      if (charCodeAt(resolved, 0) === BACKSLASH) return '\\\\?\\UNC\\' + slice(resolved, 2);
      return '\\\\?\\' + resolved;
    }

    const posix = freeze({
      sep: '/',
      delimiter: ':',
      join: joinPosix,
      normalize: normalizePosix,
      basename: basenamePosix,
      dirname: dirnamePosix,
      extname: extnamePosix,
      isAbsolute: isAbsolutePosix,
      parse: parsePosix,
      format: formatPosix,
      common: (paths) => commonPath(paths, '/'),
      resolve: resolvePosix,
      relative: relativePosix,
      toNamespacedPath: assertPath,
    });

    const win32 = freeze({
      sep: '\\',
      delimiter: ';',
      join: joinWin32,
      normalize: normalizeWin32,
      basename: basenameWin32,
      dirname: dirnameWin32,
      extname: extnameWin32,
      isAbsolute: isAbsoluteWin32,
      parse: parseWin32,
      format: formatWin32,
      common: (paths) => commonPath(paths, '\\'),
      resolve: resolveWin32,
      relative: relativeWin32,
      toNamespacedPath: toNamespacedPathWin32,
    });

    const native = hostWindows ? win32 : posix;
    return freeze({
      posix,
      win32,
      join: native.join,
      normalize: native.normalize,
      basename: native.basename,
      dirname: native.dirname,
      extname: native.extname,
      isAbsolute: native.isAbsolute,
      parse: native.parse,
      format: native.format,
      common: native.common,
      resolve: native.resolve,
      relative: native.relative,
      toNamespacedPath: native.toNamespacedPath,
      SEPARATOR: native.sep,
      DELIMITER: native.delimiter,
      SEPARATOR_PATTERN: hostWindows ? /[\\/]+/ : /\/+/,
    });
})
