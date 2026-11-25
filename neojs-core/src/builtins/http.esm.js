// The `http` built-in module as seen by `import ... from "neo:http"`.
//
// The implementation lives in `neo.js` (spliced from `builtins/http.js` when
// the `neo-http` feature is enabled), and calls private host ops registered by
// `neojs-core/src/http.rs`. This file only re-exports the public values so
// every named export shares identity with `Neo.getBuiltInModule("neo:http")`.
const http = Neo.getBuiltInModule("neo:http");

export const HttpError = http.HttpError;
export const HttpRequest = http.HttpRequest;
export const HttpResponse = http.HttpResponse;
export const ServeMux = http.ServeMux;
export const createRouter = http.createRouter;
export const response = http.response;
export const json = http.json;
export const html = http.html;
export const text = http.text;
export const redirect = http.redirect;
export const noContent = http.noContent;
export const setCookie = http.setCookie;
export const httpError = http.httpError;
export const badRequest = http.badRequest;
export const unauthorized = http.unauthorized;
export const forbidden = http.forbidden;
export const notFound = http.notFound;
export const conflict = http.conflict;
export const methodNotAllowed = http.methodNotAllowed;
export const recover = http.recover;
export const requestId = http.requestId;
export const logger = http.logger;
export const securityHeaders = http.securityHeaders;
export const cors = http.cors;
export const compose = http.compose;
export const serve = http.serve;
export const parseCookies = http.parseCookies;
export const formatSetCookie = http.formatSetCookie;
