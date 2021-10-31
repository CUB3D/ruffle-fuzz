FROM rust:1.53.0
USER root
WORKDIR /home/code

RUN apt-get update
ENV DISPLAY :0
RUN DEBIAN_FRONTEND=noninteractive apt-get install -y xserver-xorg-video-dummy pkgconf libasound2-dev libssl-dev libxcb-shape0-dev libxcb-xfixes0-dev build-essential libnss3 libnspr4 libgtk2.0-0 libpango-1.0-0 libfontconfig1 xorg libxcursor1
RUN dpkg --add-architecture i386
RUN apt-get update
RUN DEBIAN_FRONTEND=noninteractive apt-get install -y libc6:i386 libncurses5:i386 libstdc++6:i386 multiarch-support libasound2-dev:i386 libssl-dev:i386 libxcb-shape0-dev:i386 libxcb-xfixes0-dev:i386  libnss3:i386 libnspr4:i386 libgtk2.0-0:i386 libpango-1.0-0:i386 libfontconfig1:i386 libxcursor1:i386 libxt-dev:i386

COPY ./docker-fuzz/xorg.conf ./xorg.conf
COPY ./docker-fuzz/entry.sh ./entry.sh
COPY ./docker-fuzz/mm.cfg /root/mm.cfg

COPY ./ruffle ./ruffle/

COPY Cargo.lock ./
COPY Cargo.toml ./
COPY ./utils ./utils/
COPY ./swf ./swf/
COPY ./ptrace ./ptrace/
COPY ./src ./src
RUN cargo build --release

CMD ["sh", "entry.sh"]
