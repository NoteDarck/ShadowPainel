# Pesquisa da integração Playit.gg

- https://playit.gg/ — O Playit orienta instalar o agente na mesma máquina do servidor, escolher o jogo e a porta local e compartilhar o endereço público. A página informa que o tráfego é retransmitido sem exigir redirecionamento de portas no roteador; Minecraft Java é um dos jogos suportados.
- https://playit.gg/support/run-on-linux/ — A documentação oficial descreve instalação por APT em Ubuntu/Debian e também o uso de binário oficial baixado, com `chmod +x` antes da execução.
- https://github.com/playit-cloud/playit-agent/blob/master/README.md — README oficial do agente: releases atuais 2.0.x; o agente pode ser instalado por pacote/binário; no Windows e macOS pode operar como serviço; em Docker usa `SECRET_KEY`/`SECRET_KEY_FILE`; recomenda usar apenas fontes oficiais.
- https://playit.gg/support/minecraft-java-unknown-host/ — A conexão Java usa o endereço público do túnel; problemas de DNS podem causar `Unknown host` e a documentação lista ajustes específicos por sistema operacional.

Decisão de implementação no ShadowPainel 0.10.1: controlar apenas o processo local do agente (`playit`/`playit.exe` ou caminho informado pelo usuário), detectar status por PID e fornecer link para download oficial. O painel não implementa API privada do Playit nem armazena conta, segredo do agente ou endereço público; o usuário configura o túnel Minecraft Java no painel oficial apontando para `127.0.0.1:<porta>`. Isso mantém credenciais fora do ShadowPainel e segue a separação entre agente local e configuração de túneis documentada pelo Playit.
