# A closure as an HTTP route handler and as middleware.
#
# The router stores an environment for each and the server passes it back, but
# registration never received one: the closure was boxed and the box handed
# over as the function pointer, which crashed the server on the first request.

fn main() do
  let body = "{\"from\":\"closure-${40 + 2}\"}"
  let stamp = "stamped-${3 + 4}"
  let r = HTTP.router()
  let r = HTTP.use(r, fn (request, next) do
    if Request.path(request) == "/stamp" do
      HTTP.response(200, stamp)
    else
      next(request)
    end
  end)
  let r = HTTP.route(r, "/closure", fn (request) -> HTTP.response(200, body) end)
  HTTP.serve(r, 18080)
end
