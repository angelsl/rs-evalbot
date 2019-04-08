defmodule ElixirEval do
  require Logger

  def start do
    case :gen_tcp.listen(0, [:binary, :local, fd: 3, active: false]) do
      {:ok, socket} -> accept_loop(socket)
      {:error, reason} -> Logger.error("Unable to listen: #{reason}")
    end
  end

  defp accept_loop(socket, buffer \\ <<>>, bindings \\ %{}) do
    case :gen_tcp.accept(socket) do
      {:ok, client} ->
        {new_buffer, new_bindings} = serve(client, buffer, bindings)
        accept_loop(socket, new_buffer, new_bindings)

      {:error, reason} ->
        Logger.error("Unable to accept connection: #{reason}")
    end
  end

  defp serve(socket, buffer, bindings) do
    case :gen_tcp.recv(socket, 0) do
      {:ok, input} ->
        interpret(socket, buffer <> input, bindings)

      {:error, reason} ->
        Logger.error("Error serving: #{reason}")
        {<<>>, bindings}
    end
  end

  defp interpret(socket, buffer, bindings) do
    with <<_timeout::native-32, context_size::native-32, code_size::native-32, buffer::binary>> <-
           buffer,
         <<context::binary-size(context_size), code::binary-size(code_size), buffer::binary>> <-
           buffer do
      new_bindings = eval(socket, context, code, bindings)
      {buffer, new_bindings}
    else
      _ -> {buffer, bindings}
    end
  end

  defp eval(socket, context, code, bindings) do
    try do
      binding = bindings[context] || []
      {result, binding} = Code.eval_string(code, binding)
      respond(socket, inspect(result))
      Map.put(bindings, context, binding)
    rescue
      exception ->
        formatted_message = Exception.format(:error, exception)
        respond(socket, formatted_message)
        bindings
    end
  end

  defp respond(socket, response) when is_binary(response) do
    :gen_tcp.send(socket, <<byte_size(response)::native-32, response::binary>>)
  end
end

ElixirEval.start()
